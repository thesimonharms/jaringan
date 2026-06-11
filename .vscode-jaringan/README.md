# Jaringan — VS Code Syntax Highlighting

Provides syntax highlighting for **Jaringan** (`.jrg`) files in Visual Studio Code.

## Features

- **Metadata separator** — Highlights `~~~~~` separators and key:value metadata pairs.
- **Headings** — Lines starting with `#` through `######` styled as section headings.
- **Target links** — `=> target Label` rendered as markup links.
- **Actions** — `! action ...` buttons highlighted as keywords.
- **Inputs** — `? input ...` fields highlighted as keywords.
- **Images** — `@ image ...` tags highlighted as keywords.
- **Lists** — `- list items` styled as markup list entries.
- **Quotes** — `> quoted text` styled as markup quotes.
- **Horizontal rules** — `---` rendered as a thematic break.
- **Tables** — `| table | rows |` styled as markup tables.
- **Fenced code blocks** — Triple-backtick code blocks with optional language tag.
- **Comments** — Line comments (`//`) and block comments (`/* */`).
- **Strings & numbers** — Double, single, and backtick quoted strings; decimal, hex, binary, and octal numbers.

## Installation

Copy the `.vscode-jaringan/` folder into your VS Code extensions directory, or package it as a `.vsix`:

```bash
npx vsce package
```

Then install via the Extensions view: `Install from VSIX...`

## Usage

Open any `.jrg` file, and the grammar will apply automatically. If the language is not detected, select **Jaringan** from the language picker.

## License

MIT
