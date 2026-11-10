# How to write replacements

Replacements fix words the speech model keeps getting wrong, such as "nate builds" for n8builds.
They live in one plain text file, `replacements.txt`, in Scribe's data folder (`%APPDATA%\com.natkins.scribe\` on Windows).
Words > Replacements > Open file opens it in your text editor, and Scribe picks up your changes within a couple of seconds.

## Format

```text
# My replacements
nate builds -> n8builds
clawed -> Claude
superbase -> Supabase
```

- Write one replacement per line: what the model wrote, then `->`, then what it should be.
- Lines starting with `#` are comments, and blank lines are ignored.
- A line without `->` is skipped, and Scribe tells you its line number.
- Scribe also reads `=>` and `→` as the arrow.

## How they apply

- Matching ignores case: `clawed` also fixes "Clawed".
- Only whole words match: `no` never touches "nope".
- Longer phrases go first, so "cloud code -> Claude Code" wins over "cloud -> Claude".
- Phrases of the same length go in file order.
- Each replacement works on the text the ones before it produced.
- They run after filler removal, with every model, and there is no limit on how many you have.

## What to add

- Add real mishearings only, where letters change: `nate builds -> n8builds`, `versal -> Vercel`.
- Skip spacing, hyphen and case variants such as "mono repo" or "Next js": they already read right.
- For names the model misses in different ways each time, use Custom words instead.

## Adding from the app

- Add one appends a single line to the file.
- Paste a list and Import file read the same format, also accepting `=`, a tab or a comma, and dropping bullets, numbering and quotes.
- Before anything is added, they show which entries are new and which you already have.
- The list of every replacement shows four rows at a time and scrolls for more; Find a replacement narrows it, and each entry can be removed.
- Format opens a short summary with an example, and links to this guide.
- Copy all copies the whole file's replacements in this format.

## Upgrading from 0.15

The first time Scribe 0.15.1 starts, it moves the replacements kept in its settings into `replacements.txt`, in the same order.
It keeps a copy of that list next to the file, as `replacements-backup-from-settings.json`.
With GitHub backup on, `replacements.txt` is backed up with your notes as `scribe/replacements.txt` whenever it changes.
