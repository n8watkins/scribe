// How replacements.txt is written, for the Format dialog.
// Keep in step with docs/REPLACEMENTS.md and src-tauri/src/text_replace.rs.

export const FORMAT_RULES: readonly string[] = [
  "One replacement per line, written as heard\u00a0->\u00a0written.",
  "Lines starting with # are comments; blank lines are ignored.",
  "Matching ignores case and only matches whole words.",
  "Longer heard phrases are applied first; equal lengths go in file order.",
  "Each replacement works on the text the ones before it produced.",
  "Add only real mishearings, where letters change, such as nate builds to n8builds. Skip spacing, hyphen and case variants such as mono repo or Next js: they already read right.",
];
