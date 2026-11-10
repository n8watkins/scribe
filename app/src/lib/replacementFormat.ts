// How replacements.txt is written, for the Format panel and the AI prompt.
// Keep in step with docs/REPLACEMENTS.md and src-tauri/src/text_replace.rs.

export const FORMAT_RULES: readonly string[] = [
  "One replacement per line: heard -> written.",
  "Lines starting with # are comments; blank lines are ignored.",
  "Matching ignores case and only matches whole words.",
  "Longer heard phrases are applied first; equal lengths go in file order.",
  "Each replacement works on the text the ones before it produced.",
  "Add only real mishearings, where letters change (nate builds -> n8builds). Skip spacing, hyphen and case variants (mono repo, Next js): they already read right.",
];

export const AI_PROMPT = `Convert the list below into Scribe replacements.

Rules:
- Output only lines of the form: heard -> written
- One replacement per line, nothing else (no numbering, bullets, quotes or commentary).
- "heard" is what the speech model wrote; "written" is what it should be.
- Drop entries where heard and written differ only in spacing, hyphens, case or punctuation (for example "mono repo -> monorepo" or "next js -> Next.js"); keep only real mishearings, where letters change.
- Keep the written side exactly as given, including its capitals.
- If the same heard phrase appears twice, keep the first.

List:
`;
