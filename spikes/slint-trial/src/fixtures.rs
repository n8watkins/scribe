//! Fixture data: 2,000 history rows in many scripts and a 60,000-character transcript.

pub const ROWS: usize = 2000;
pub const TRANSCRIPT_CHARS: usize = 60_000;

/// One sentence per script, each short enough that one copy is one line and
/// three copies wrap to about three lines in the history list.
const SAMPLES: &[(&str, &str)] = &[
    (
        "English",
        "Remind me to send the quarterly numbers to Dana before the call on Thursday.",
    ),
    (
        "Arabic",
        "ذكّرني بإرسال أرقام الربع إلى دانا قبل المكالمة يوم الخميس.",
    ),
    (
        "Hebrew",
        "תזכיר לי לשלוח את המספרים הרבעוניים לדנה לפני השיחה ביום חמישי.",
    ),
    (
        "Mixed RTL and LTR",
        "הפגישה עם Microsoft Teams נקבעה ל-3:30 PM, ואז نراجع ملف report_v2.pdf معًا.",
    ),
    (
        "Japanese",
        "木曜日の電話の前に、四半期の数字をダナに送るようにリマインドしてください。",
    ),
    ("Chinese", "提醒我在星期四通话之前把季度数据发给达娜。"),
    (
        "Korean",
        "목요일 통화 전에 다나에게 분기 실적을 보내라고 알려 주세요.",
    ),
    (
        "Hindi",
        "गुरुवार की कॉल से पहले डाना को तिमाही के आंकड़े भेजने की याद दिलाना।",
    ),
    ("Thai", "เตือนฉันให้ส่งตัวเลขรายไตรมาสให้ดาน่าก่อนการโทรวันพฤหัสบดี"),
    (
        "Russian",
        "Напомни мне отправить квартальные цифры Дане до звонка в четверг.",
    ),
    (
        "Emoji",
        "Shipped it 🚀 thanks team 🙌 next: coffee ☕ and the demo 🎬 at 3 🕒 👍🏽 👨‍👩‍👧‍👦 🇯🇵",
    ),
];

pub struct Row {
    pub time: String,
    pub lang: &'static str,
    pub preview: String,
}

/// The font family for a transcript in a CJK language. Slint's fallback has
/// no language: with the default chain, Han characters fall back to a
/// Japanese font that lacks simplified Chinese glyphs (话 发 给 show as
/// boxes). Scribe knows each transcript's language, so rows name the font.
pub fn family_for(lang: &str) -> &'static str {
    if cfg!(windows) {
        match lang {
            "Chinese" => "Microsoft YaHei UI",
            "Japanese" => "Yu Gothic UI",
            "Korean" => "Malgun Gothic",
            _ => "",
        }
    } else {
        ""
    }
}

/// Deterministic rows: script by index, one to three copies of its sentence.
pub fn history() -> Vec<Row> {
    (0..ROWS)
        .map(|i| {
            let (lang, text) = SAMPLES[i % SAMPLES.len()];
            let copies = 1 + (i / SAMPLES.len()) % 3;
            let preview = vec![text; copies].join(" ");
            let minutes = (ROWS - i) * 17;
            let time = format!(
                "Nov {:02}, {:02}:{:02}",
                1 + (minutes / (24 * 60)) % 28,
                (minutes / 60) % 24,
                minutes % 60
            );
            Row {
                time,
                lang,
                preview,
            }
        })
        .collect()
}

const PARAGRAPH: &str = "So the plan for tomorrow is to finish the assessment, send the notes to the team and then look at the installer again. The upgrade test failed twice last week because the old service was still holding the file, and I want to know whether waiting for it to stop is enough or whether we need to ask Windows to replace the file on the next restart. After lunch I will record the demo, two takes at most, and keep the cursor slow.";

const RTL_PARAGRAPH: &str = "هذه فقرة باللغة العربية داخل النص الطويل، فيها رقم 2026 وكلمة English في الوسط، ويجب أن تُعرض من اليمين إلى اليسار بالترتيب الصحيح. וזו פסקה בעברית עם המספר 42 והמילה Scribe באמצע.";

/// About 60,000 characters of dictation, with a right-to-left paragraph near
/// the top (visible without scrolling) and every tenth paragraph after it.
pub fn transcript() -> String {
    let mut out = String::with_capacity(TRANSCRIPT_CHARS * 2);
    let mut n = 0usize;
    let mut chars = 0usize;
    while chars < TRANSCRIPT_CHARS {
        let p = if n == 1 || (n > 1 && n.is_multiple_of(10)) {
            RTL_PARAGRAPH
        } else {
            PARAGRAPH
        };
        let take = (TRANSCRIPT_CHARS - chars).min(p.chars().count());
        let piece: String = p.chars().take(take).collect();
        chars += take;
        out.push_str(&piece);
        if chars < TRANSCRIPT_CHARS {
            out.push_str("\n\n");
            chars += 2;
        }
        n += 1;
    }
    out
}

/// True when the first strongly directional character is Hebrew or Arabic.
/// Slint's `horizontal-alignment: start` does not follow the paragraph
/// direction in a `Text` (it does in `TextEdit`), so rows pick right or left.
pub fn starts_rtl(text: &str) -> bool {
    for c in text.chars() {
        let u = c as u32;
        if matches!(u, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF) {
            return true;
        }
        if c.is_alphabetic() {
            return false;
        }
    }
    false
}

/// Words the fake recogniser "hears", one at a time, for the live text.
pub const LIVE_WORDS: &str = "So the plan for tomorrow is to finish the assessment and send the notes to the team, then look at the installer again because the upgrade test failed twice last week and I want to know why before Friday";

/// The confirmed transcript the confirmation state shows.
pub const CONFIRMED: &str =
    "So the plan for tomorrow is to finish the assessment and send the notes to the team.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_is_exactly_sixty_thousand_characters() {
        assert_eq!(transcript().chars().count(), TRANSCRIPT_CHARS);
    }

    #[test]
    fn history_has_every_script_and_one_to_three_copies() {
        let rows = history();
        assert_eq!(rows.len(), ROWS);
        for (lang, _) in SAMPLES {
            assert!(rows.iter().any(|r| r.lang == *lang));
        }
        let max = rows
            .iter()
            .map(|r| r.preview.matches(SAMPLES[0].1).count())
            .max()
            .unwrap();
        assert_eq!(max, 3);
    }
}
