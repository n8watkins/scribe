use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL_ID: &str = "small.en-q5_1";
const HUGGING_FACE_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const HUGGING_FACE_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve";

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
    Sha256,
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
    pub expected_sha256: &'static str,
    pub expected_size_bytes: u64,
    /// Whether this Whisper model can transcribe languages other than English
    /// (and translate to English). The `.en` Whisper builds are English-only;
    /// the plain (non-`.en`) builds and `large-v3-turbo` are multilingual. The
    /// UI uses this to label rows and to warn when a non-English language or
    /// translate is selected against an English-only model.
    pub multilingual: bool,
}

impl CatalogModel {
    pub fn download_url(self) -> String {
        format!(
            "{}/{}/{}",
            HUGGING_FACE_BASE_URL, HUGGING_FACE_REVISION, self.filename
        )
    }

    pub fn checksum(self) -> Option<ModelChecksum> {
        Some(ModelChecksum {
            kind: ChecksumKind::Sha256,
            value: self.expected_sha256.to_string(),
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
            expected_sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
            expected_size_bytes: 77_704_715,
            multilingual: false,
        },
        CatalogModel {
            id: "base.en",
            name: "Base English",
            filename: "ggml-base.en.bin",
            disk_size_label: "142 MiB",
            expected_sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            expected_size_bytes: 147_964_211,
            multilingual: false,
        },
        CatalogModel {
            id: "small.en",
            name: "Small English",
            filename: "ggml-small.en.bin",
            disk_size_label: "466 MiB",
            expected_sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
            expected_size_bytes: 487_614_201,
            multilingual: false,
        },
        CatalogModel {
            id: DEFAULT_MODEL_ID,
            name: "Small English Q5_1",
            filename: "ggml-small.en-q5_1.bin",
            disk_size_label: "181 MiB",
            expected_sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
            expected_size_bytes: 190_098_681,
            multilingual: false,
        },
        CatalogModel {
            id: "medium.en",
            name: "Medium English",
            filename: "ggml-medium.en.bin",
            disk_size_label: "1.5 GiB",
            expected_sha256: "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
            expected_size_bytes: 1_533_774_781,
            multilingual: false,
        },
        // Multilingual Whisper builds. These transcribe ~99 languages and can
        // translate any of them to English (the Translate-to-English setting).
        // Every download is pinned to a reviewed repository revision and checked
        // against the plain-content SHA-256 and byte size published in its LFS
        // metadata.
        CatalogModel {
            id: "tiny",
            name: "Tiny (Multilingual)",
            filename: "ggml-tiny.bin",
            disk_size_label: "75 MiB",
            expected_sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            expected_size_bytes: 77_691_713,
            multilingual: true,
        },
        CatalogModel {
            id: "base",
            name: "Base (Multilingual)",
            filename: "ggml-base.bin",
            disk_size_label: "142 MiB",
            expected_sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            expected_size_bytes: 147_951_465,
            multilingual: true,
        },
        CatalogModel {
            id: "small",
            name: "Small (Multilingual)",
            filename: "ggml-small.bin",
            disk_size_label: "466 MiB",
            expected_sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            expected_size_bytes: 487_601_967,
            multilingual: true,
        },
        CatalogModel {
            id: "medium",
            name: "Medium (Multilingual)",
            filename: "ggml-medium.bin",
            disk_size_label: "1.5 GiB",
            expected_sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
            expected_size_bytes: 1_533_763_059,
            multilingual: true,
        },
        CatalogModel {
            id: "large-v3-turbo-q5_0",
            name: "Large v3 Turbo Q5_0 (Multilingual)",
            filename: "ggml-large-v3-turbo-q5_0.bin",
            disk_size_label: "547 MiB",
            expected_sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
            expected_size_bytes: 574_041_195,
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
            expected_sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
            expected_size_bytes: 1_624_555_275,
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
            // The plain (non-`.en`) builds resolve to the matching Hugging Face
            // file and have a full content hash plus an exact size bound.
            assert_eq!(model.filename, format!("ggml-{id}.bin"));
            assert_eq!(model.expected_sha256.len(), 64);
            assert!(model.expected_size_bytes > 0);
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
        assert_eq!(q5.expected_sha256.len(), 64);
        assert_eq!(full.expected_sha256.len(), 64);
        assert!(full.expected_size_bytes > q5.expected_size_bytes);
    }

    #[test]
    fn download_urls_point_at_the_whisper_cpp_repo() {
        let small = catalog_model("small").unwrap();
        assert_eq!(
            small.download_url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin"
        );
    }

    #[test]
    fn every_catalog_download_has_a_sha256_and_size_bound() {
        for model in catalog() {
            assert_eq!(model.expected_sha256.len(), 64, "{} hash", model.id);
            assert!(
                model
                    .expected_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
                "{} hash must be hexadecimal",
                model.id
            );
            assert!(model.expected_size_bytes > 0, "{} size", model.id);
            assert_eq!(model.checksum().unwrap().kind, ChecksumKind::Sha256);
        }
    }
}
