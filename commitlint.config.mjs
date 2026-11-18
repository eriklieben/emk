// Commit message rules: the Angular format, `type(scope): subject`, checked by a local commit-msg hook after
// `pnpm install` in the repository root. Details and examples: docs/development.md, "Commit messages".
//
//   echo "feat(display): add a battery screen" | pnpm exec commitlint
//
// config-conventional = the Angular types (build, ci, docs, feat, fix, perf, refactor, style, test) plus chore and
// revert, and also accepts the `!` breaking-change marker (`feat(keymap)!: ...`), which release.config.mjs understands.
export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    // Suggested scopes: a warning, not an error, and the scope stays optional. Keep in sync with docs/development.md.
    "scope-enum": [
      1,
      "always",
      [
        // Firmware and its crates
        "core", "keymap", "display", "ble", "fw-nrf", "power", "mouse", "studio", "bootloader",
        // Host side
        "app", "emulator", "kbnotify", "sim", "emu",
        // The repository
        "docs", "ci", "deps", "deps-dev", "release", "tools", "hil", "bench",
      ],
    ],
    // Bodies and footers hold URLs, stack traces and generated release notes (semantic-release): no line length
    // limit there. The header (the line that ends up in the changelog) stays at 100 characters.
    "body-max-line-length": [0],
    "footer-max-line-length": [0],
  },
};
