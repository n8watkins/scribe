// scribe-parakeet: Scribe's resident Parakeet engine.
//
// Loads an NVIDIA Parakeet TDT model once (through whisper.cpp's libparakeet,
// so it uses the same ggml CPU, Vulkan and Metal backends as whisper-server)
// and then transcribes WAV files on request until Scribe goes away.
//
// Protocol: one JSON object per line.
//   stdout, once, after loading:  {"ready":true,"loadMs":812}
//                     or:         {"ready":false,"error":"..."}  (then exits 1)
//   stdin, per request:           {"wav":"C:\\path\\to\\clip.wav"}
//   stdout, per request:          {"ok":true,"text":"...","words":[{"w":"And","s":240,"e":560}],"ms":95}
//                     or:         {"ok":false,"error":"..."}
// Word times are in milliseconds.
//
// It never outlives Scribe: it exits when its stdin closes (Scribe holds the
// other end, and the OS closes it when Scribe dies for any reason) and when
// the process given by --parent-pid is gone.

#include "parakeet.h"
#include "ggml-backend.h"
#include "common-whisper.h"
#include "json.hpp"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <string>
#include <thread>
#include <vector>

#ifdef _WIN32
#    define WIN32_LEAN_AND_MEAN
#    define NOMINMAX
#    include <windows.h>
#    include <fcntl.h>
#    include <io.h>
#else
#    include <unistd.h>
#endif

using json = nlohmann::ordered_json;

namespace {

struct options {
    std::string model;
    int  threads    = 4;
    bool use_gpu    = true;
    int  gpu_device = 0;
    long parent_pid = 0;
};

// Longest stretch handed to the model at once. Longer audio is cut at the
// quietest moment near the end of each window, so a word is rarely split.
constexpr int SAMPLE_RATE      = PARAKEET_SAMPLE_RATE;
constexpr int MAX_CHUNK        = 30 * SAMPLE_RATE;
constexpr int CUT_SEARCH       = 8 * SAMPLE_RATE;
constexpr int CUT_FRAME        = SAMPLE_RATE / 50; // 20 ms

void quiet_log(enum ggml_log_level, const char *, void *) {}

void emit(const json & message) {
    std::string line = message.dump(-1, ' ', false, json::error_handler_t::replace);
    line.push_back('\n');
    fwrite(line.data(), 1, line.size(), stdout);
    fflush(stdout);
}

bool parse_args(int argc, char ** argv, options & opts) {
    for (int i = 1; i < argc; i++) {
        const std::string arg = argv[i];
        auto next = [&]() -> const char * { return i + 1 < argc ? argv[++i] : nullptr; };
        if (arg == "--model" || arg == "-m") {
            const char * v = next(); if (!v) return false; opts.model = v;
        } else if (arg == "--threads" || arg == "-t") {
            const char * v = next(); if (!v) return false; opts.threads = std::max(1, std::atoi(v));
        } else if (arg == "--no-gpu" || arg == "-ng") {
            opts.use_gpu = false;
        } else if (arg == "--device" || arg == "-dev") {
            const char * v = next(); if (!v) return false; opts.gpu_device = std::atoi(v);
        } else if (arg == "--parent-pid") {
            const char * v = next(); if (!v) return false; opts.parent_pid = std::atol(v);
        } else if (arg == "--help" || arg == "-h") {
            fprintf(stderr, "usage: %s --model FILE [--threads N] [--no-gpu] [--device N] [--parent-pid PID]\n", argv[0]);
            exit(0);
        } else {
            fprintf(stderr, "unknown argument: %s\n", arg.c_str());
            return false;
        }
    }
    return !opts.model.empty();
}

// Exits the moment the parent process is gone, in case stdin stays open
// (for example when a grandchild inherited it).
void watch_parent(long parent_pid) {
    if (parent_pid <= 0) {
        return;
    }
#ifdef _WIN32
    HANDLE parent = OpenProcess(SYNCHRONIZE, FALSE, (DWORD) parent_pid);
    if (parent == nullptr) {
        _exit(0); // already gone
    }
    std::thread([parent]() {
        WaitForSingleObject(parent, INFINITE);
        _exit(0);
    }).detach();
#else
    if (getppid() != (pid_t) parent_pid) {
        _exit(0);
    }
    std::thread([parent_pid]() {
        while (getppid() == (pid_t) parent_pid) {
            std::this_thread::sleep_for(std::chrono::milliseconds(250));
        }
        _exit(0);
    }).detach();
#endif
}

// The end of the next chunk starting at `start`: the quietest 20 ms frame in
// the last few seconds of a 30 s window, or the end of the audio.
size_t chunk_end(const std::vector<float> & pcm, size_t start) {
    if (pcm.size() - start <= (size_t) MAX_CHUNK) {
        return pcm.size();
    }
    const size_t window_end   = start + MAX_CHUNK;
    const size_t search_start = window_end - CUT_SEARCH;
    size_t best = window_end;
    double best_energy = INFINITY;
    for (size_t frame = search_start; frame + CUT_FRAME <= window_end; frame += CUT_FRAME) {
        double energy = 0.0;
        for (size_t i = frame; i < frame + CUT_FRAME; i++) {
            energy += (double) pcm[i] * pcm[i];
        }
        if (energy < best_energy) {
            best_energy = energy;
            best = frame + CUT_FRAME / 2;
        }
    }
    return best;
}

struct word {
    std::string text;
    int64_t start_ms;
    int64_t end_ms;
};

std::string trim(const std::string & s) {
    const auto first = s.find_first_not_of(" \t\r\n");
    if (first == std::string::npos) {
        return "";
    }
    const auto last = s.find_last_not_of(" \t\r\n");
    return s.substr(first, last - first + 1);
}

// Transcribes one chunk and appends its text and words, shifting times by
// `offset_ms`. Returns false on a model error.
bool transcribe_chunk(parakeet_context * ctx, const options & opts, const float * samples, int n_samples,
                      int64_t offset_ms, std::string & text, std::vector<word> & words) {
    parakeet_full_params params = parakeet_full_default_params(PARAKEET_SAMPLING_GREEDY);
    params.n_threads  = opts.threads;
    params.no_context = true;

    if (parakeet_full(ctx, params, samples, n_samples) != 0) {
        return false;
    }

    const int n_segments = parakeet_full_n_segments(ctx);
    for (int s = 0; s < n_segments; s++) {
        const std::string segment = trim(parakeet_full_get_segment_text(ctx, s));
        if (!segment.empty()) {
            if (!text.empty()) {
                text.push_back(' ');
            }
            text += segment;
        }

        const int n_tokens = parakeet_full_n_tokens(ctx, s);
        for (int t = 0; t < n_tokens; t++) {
            const parakeet_token_data data = parakeet_full_get_token_data(ctx, s, t);
            const char * piece = parakeet_token_to_str(ctx, data.id);
            if (piece == nullptr) {
                continue;
            }
            char buffer[256];
            parakeet_token_to_text(piece, false, buffer, sizeof(buffer));
            const std::string token_text = trim(buffer);
            if (token_text.empty()) {
                continue;
            }
            // Token times are in 10 ms units.
            const int64_t start = offset_ms + data.t0 * 10;
            const int64_t end   = offset_ms + std::max(data.t1, data.t0) * 10;
            if (data.is_word_start || words.empty()) {
                words.push_back({token_text, start, end});
            } else {
                words.back().text += token_text;
                words.back().end_ms = std::max(words.back().end_ms, end);
            }
        }
    }
    return true;
}

json handle(parakeet_context * ctx, const options & opts, const json & request) {
    if (!request.contains("wav") || !request["wav"].is_string()) {
        return {{"ok", false}, {"error", "The request has no \"wav\" path."}};
    }
    const std::string path = request["wav"].get<std::string>();

    std::vector<float> pcm;
    std::vector<std::vector<float>> stereo;
    if (!read_audio_data(path, pcm, stereo, false)) {
        return {{"ok", false}, {"error", "Could not read the audio file " + path}};
    }

    const auto started = std::chrono::steady_clock::now();
    std::string text;
    std::vector<word> words;
    size_t start = 0;
    while (start < pcm.size()) {
        const size_t end = chunk_end(pcm, start);
        // Shorter than 0.1 s: nothing a model can hear.
        if (end - start >= (size_t) (SAMPLE_RATE / 10)) {
            const int64_t offset_ms = (int64_t) start * 1000 / SAMPLE_RATE;
            if (!transcribe_chunk(ctx, opts, pcm.data() + start, (int) (end - start), offset_ms, text, words)) {
                return {{"ok", false}, {"error", "Parakeet could not transcribe the audio."}};
            }
        }
        start = end;
    }
    const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - started).count();

    json word_list = json::array();
    for (const auto & w : words) {
        word_list.push_back({{"w", w.text}, {"s", w.start_ms}, {"e", w.end_ms}});
    }
    return {{"ok", true}, {"text", text}, {"words", word_list}, {"ms", ms}};
}

} // namespace

int main(int argc, char ** argv) {
#ifdef _WIN32
    // Byte-exact lines both ways; no CRLF translation.
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
#endif
    options opts;
    if (!parse_args(argc, argv, opts)) {
        emit({{"ready", false}, {"error", "usage: scribe-parakeet --model FILE [--threads N] [--no-gpu] [--device N] [--parent-pid PID]"}});
        return 2;
    }
    watch_parent(opts.parent_pid);

    parakeet_log_set(quiet_log, nullptr);
    ggml_backend_load_all();

    const auto load_started = std::chrono::steady_clock::now();
    parakeet_context_params ctx_params = parakeet_context_default_params();
    ctx_params.use_gpu    = opts.use_gpu;
    ctx_params.gpu_device = opts.gpu_device;
    parakeet_context * ctx = parakeet_init_from_file_with_params(opts.model.c_str(), ctx_params);
    if (ctx == nullptr) {
        emit({{"ready", false}, {"error", "Could not load the Parakeet model " + opts.model}});
        return 1;
    }
    const auto load_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - load_started).count();
    emit({{"ready", true}, {"loadMs", load_ms}});

    std::string line;
    while (std::getline(std::cin, line)) {
        if (!line.empty() && line.back() == '\r') {
            line.pop_back();
        }
        if (line.empty()) {
            continue;
        }
        json request = json::parse(line, nullptr, false);
        if (request.is_discarded()) {
            emit({{"ok", false}, {"error", "The request is not JSON."}});
            continue;
        }
        emit(handle(ctx, opts, request));
    }

    // stdin closed: Scribe is gone or asked us to stop.
    parakeet_free(ctx);
    return 0;
}
