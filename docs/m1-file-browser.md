# M1 File Browser Notes

M1 turns Jaringan from a renderer demo into a local, navigable terminal browser.

## Decisions from product feedback

- Page files use `.jrg`, not `.jar`. Java archive files are a large existing standard and are often treated as suspicious by security tools.
- Network URLs use the `jrg://` scheme, matching the file identity and avoiding `jar://` ambiguity.
- The terminal UI should be aesthetically intentional, not merely functional: borders, color, status text, loading animations, selected-link highlighting, and graceful error/status messages are part of the product.
- Pages should support interactive controls and media declarations from the beginning, even if early support is simple.

## M1 scope

- `jaringan-browser open <path>` launches a ratatui TUI.
- Local relative `.jrg` links navigate between files.
- Back navigation returns to the previous page.
- Unsupported targets show a status message rather than crashing.
- Buttons render as selectable actions and show status text when activated.
- Images render as terminal-native placeholders. Local images are detected; remote HTTP/HTTPS image URLs are downloaded into `~/.cache/jaringan/images/` when activated.
- Loading feedback is displayed during page transitions; the initial implementation may be brief because local files load quickly.
- Browser interaction is modal with exactly two movement modes:
  - **Selection mode**: `j`/`k` or arrows move between selectable links, buttons, and images.
  - **Scroll mode**: `j`/`k` or arrows scroll the document viewport.
  - `tab` toggles modes, `s` enters scroll mode, and `v` enters selection mode.

## Draft page syntax additions

```text
# Home

=> about.jrg About page
! refresh label="Refresh recommendations" target="refresh"
@ ./cover.png alt="Cover image"
```

- `=> target label` is a navigable link.
- `! id label="..." target="..."` is a button/action.
- `@ source alt="..."` is an image declaration.
