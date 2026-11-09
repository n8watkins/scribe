# Scribe's resident Parakeet engine (see scribe-parakeet.cpp).
#
# Built inside the reviewed whisper.cpp source tree, next to whisper-cli and
# whisper-server, so it uses the same ggml backends and build flags. The
# workflows add it with one line at the end of whisper.cpp's
# examples/CMakeLists.txt:
#
#   add_subdirectory(<repo>/app/sidecar/scribe-parakeet ${CMAKE_BINARY_DIR}/scribe-parakeet)
set(TARGET scribe-parakeet)
add_executable(${TARGET} scribe-parakeet.cpp)

include(DefaultTargetOptions)

target_link_libraries(${TARGET} PRIVATE common json_cpp parakeet ${CMAKE_THREAD_LIBS_INIT})

install(TARGETS ${TARGET} RUNTIME)
