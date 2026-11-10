// How replacements.txt is written, for the Format dialog.
// Keep in step with docs/REPLACEMENTS.md and src-tauri/src/text_replace.rs.

export const FORMAT_RULES: readonly string[] = [
  "One replacement per line: heard -> written.",
  "Lines starting with # are comments; blank lines are ignored.",
  "Matching ignores case and only matches whole words.",
  "Longer heard phrases are applied first; equal lengths go in file order.",
  "Each replacement works on the text the ones before it produced.",
  "Add only real mishearings, where letters change (nate builds -> n8builds). Skip spacing, hyphen and case variants (mono repo, Next js): they already read right.",
];
