use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL_ID: &str = "small.en-q5_1";
const HUGGING_FACE_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    NotDownloaded,
    Downloading,
    Downloaded,
    Selected,
    Loaded,
    Failed,
    UpdateAvailable,
}

impl ModelStatus {
    pub fn as_db_value(self) -> &'static str {
        match self {
            Self::NotDownloaded => "not_downloaded",
            Self::Downloading => "downloading",
            Self::Downloaded => "downloaded",
            Self::Selected => "selected",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
            Self::UpdateAvailable => "update_available",
        }
    }

    pub fn from_db_value(value: &str) -> Self {
        match value {
            "downloading" => Self::Downloading,
            "downloaded" => Self::Downloaded,
            "selected" => Self::Selected,
            "loaded" => Self::Loaded,
            "failed" => Self::Failed,
            "update_available" => Self::UpdateAvailable,
            _ => Self::NotDownloaded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChecksumKind {
    Sha1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChecksum {
    pub kind: ChecksumKind,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    AppData,
    ExternalCache,
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogModel {
    pub id: &'static str,
    pub name: &'static str,
    pub filename: &'static str,
    pub disk_size_label: &'static str,
    pub expected_sha1: Option<&'static str>,
    /// Whether this Whisper model can transcribe languages other than English
    /// (and translate to English). The `.en` Whisper builds are English-only;
    /// the plain (non-`.en`) builds and `large-v3-turbo` are multilingual. The
    /// UI uses this to label rows and to warn when a non-English language or
    /// translate is selected against an English-only model.
    pub multilingual: bool,
}

impl CatalogModel {
    pub fn download_url(self) -> String {
        format!("{}/{}", HUGGING_FACE_BASE_URL, self.filename)
    }

    pub fn checksum(self) -> Option<ModelChecksum> {
        self.expected_sha1.map(|value| ModelChecksum {
            kind: ChecksumKind::Sha1,
            value: value.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub download_url: String,
    pub disk_size_label: String,
    pub local_path: Option<String>,
    pub source: Option<ModelSource>,
    pub size_bytes: Option<u64>,
    pub status: ModelStatus,
    pub checksum: Option<ModelChecksum>,
    pub selected: bool,
    pub downloaded_at: Option<DateTime<Utc>>,
    /// Mirrors `CatalogModel::multilingual` for the frontend (English-only vs
    /// multilingual labeling + the non-English pair guard).
    pub multilingual: bool,
}

#[derive(Debug, Clone)]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub local_path: Option<String>,
    pub size_bytes: Option<u64>,
    pub status: ModelStatus,
    pub checksum: Option<String>,
    pub selected: bool,
    pub downloaded_at: Option<DateTime<Utc>>,
}

pub fn catalog() -> &'static [CatalogModel] {
    &[
        // English-only (`.en`) Whisper builds. These transcribe English only and
        // cannot translate other languages.
        CatalogModel {
            id: "tiny.en",
            name: "Tiny English",
            filename: "ggml-tiny.en.bin",
            disk_size_label: "75 MiB",
            expected_sha1: Some("c78c86eb1a8faa21b369bcd33207cc90d64ae9df"),
            multilingual: false,
        },
        CatalogModel {
            id: "base.en",
            name: "Base English",
            filename: "ggml-base.en.bin",
            disk_size_label: "142 MiB",
            expected_sha1: Some("137c40403d78fd54d454da0f9bd998f78703390c"),
            multilingual: false,
        },
        CatalogModel {
            id: "small.en",
            name: "Small English",
            filename: "ggml-small.en.bin",
            disk_size_label: "466 MiB",
            expected_sha1: Some("db8a495a91d927739e50b3fc1cc4c6b8f6c2d022"),
            multilingual: false,
        },
        CatalogModel {
            id: DEFAULT_MODEL_ID,
            name: "Small English Q5_1",
            filename: "ggml-small.en-q5_1.bin",
            disk_size_label: "181 MiB",
            expected_sha1: Some("20f54878d608f94e4a8ee3ae56016571d47cba34"),
            multilingual: false,
        },
        CatalogModel {
            id: "medium.en",
            name: "Medium English",
            filename: "ggml-medium.en.bin",
            disk_size_label: "1.5 GiB",
            expected_sha1: Some("8c30f0e44ce9560643ebd10bbe50cd20eafd3723"),
            multilingual: false,
        },
        // Multilingual Whisper builds. These transcribe ~99 languages and can
        // translate any of them to English (the Translate-to-English setting).
        // No plain-content SHA1 is published for these by upstream anymore (the
        // download-ggml-model.sh script dropped its checksum table, and the
        // HuggingFace repo only exposes git blob OIDs / LFS SHA256, neither of
        // which matches the plain SHA1 this app verifies). Rather than invent a
        // value that would fail verification, the checksum is left None — the
        // schema allows it, and downloads still come over HTTPS from the same
        // trusted HuggingFace repo as the entries above.
        CatalogModel {
            id: "tiny",
            name: "Tiny (Multilingual)",
            filename: "ggml-tiny.bin",
            disk_size_label: "75 MiB",
            expected_sha1: None,
            multilingual: true,
        },
        CatalogModel {
            id: "base",
            name: "Base (Multilingual)",
            filename: "ggml-base.bin",
            disk_size_label: "142 MiB",
            expected_sha1: None,
            multilingual: true,
        },
        CatalogModel {
            id: "small",
            name: "Small (Multilingual)",
            filename: "ggml-small.bin",
            disk_size_label: "466 MiB",
            expected_sha1: None,
            multilingual: true,
        },
        CatalogModel {
            id: "medium",
            name: "Medium (Multilingual)",
            filename: "ggml-medium.bin",
            disk_size_label: "1.5 GiB",
            expected_sha1: None,
            multilingual: true,
        },
        CatalogModel {
            id: "large-v3-turbo-q5_0",
            name: "Large v3 Turbo Q5_0 (Multilingual)",
            filename: "ggml-large-v3-turbo-q5_0.bin",
            disk_size_label: "547 MiB",
            expected_sha1: Some("e050f7970618a659205450ad97eb95a18d69c9ee"),
            multilingual: true,
        },
        // Full-precision (fp16) large-v3-turbo: the most accurate model offered.
        // ~3x the q5_0 size and slow on CPU, so it's really meant for the GPU
        // (Vulkan) path — kept after the q5_0 default, which stays recommended.
        CatalogModel {
            id: "large-v3-turbo",
            name: "Large v3 Turbo (Multilingual, max accuracy)",
            filename: "ggml-large-v3-turbo.bin",
            disk_size_label: "1.6 GiB",
            expected_sha1: None,
            multilingual: true,
        },
    ]
}

pub fn catalog_model(id: &str) -> Option<CatalogModel> {
    catalog().iter().copied().find(|model| model.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_is_present_and_english_only() {
        let default = catalog_model(DEFAULT_MODEL_ID).expect("default model in catalog");
        assert!(
            !default.multilingual,
            "the default model stays English-only"
        );
    }

    #[test]
    fn catalog_includes_multilingual_base_and_small() {
        for id in ["tiny", "base", "small", "medium"] {
            let model =
                catalog_model(id).unwrap_or_else(|| panic!("{id} should be in the catalog"));
            assert!(model.multilingual, "{id} must be flagged multilingual");
            // The plain (non-`.en`) builds resolve to the matching HuggingFace
            // file and have no fabricated checksum.
            assert_eq!(model.filename, format!("ggml-{id}.bin"));
            assert!(model.expected_sha1.is_none());
        }
    }

    #[test]
    fn en_builds_are_english_only_and_large_turbo_is_multilingual() {
        for id in [
            "tiny.en",
            "base.en",
            "small.en",
            "small.en-q5_1",
            "medium.en",
        ] {
            assert!(
                !catalog_model(id).unwrap().multilingual,
                "{id} is an English-only build"
            );
        }
        assert!(
            catalog_model("large-v3-turbo-q5_0").unwrap().multilingual,
            "large-v3-turbo is multilingual"
        );
        assert!(
            catalog_model("large-v3-turbo").unwrap().multilingual,
            "full large-v3-turbo is multilingual"
        );
    }

    #[test]
    fn full_large_turbo_is_offered_alongside_the_quantized_one() {
        // Both the recommended q5_0 and the full-precision fp16 turbo are in the
        // catalog, with distinct ids and filenames.
        let q5 = catalog_model("large-v3-turbo-q5_0").unwrap();
        let full = catalog_model("large-v3-turbo").unwrap();
        assert_eq!(q5.filename, "ggml-large-v3-turbo-q5_0.bin");
        assert_eq!(full.filename, "ggml-large-v3-turbo.bin");
        // The full one has no fabricated checksum (HF publishes none that matches).
        assert!(full.expected_sha1.is_none());
    }

    #[test]
    fn download_urls_point_at_the_whisper_cpp_repo() {
        let small = catalog_model("small").unwrap();
        assert_eq!(
            small.download_url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }
}
