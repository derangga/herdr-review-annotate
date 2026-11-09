// The icon for a file in the sidebar, as the name of a drawing in `icons/` (no folder, no `.svg`). The keys are
// the TUI's `icons.rs` keys: a whole file name first, then the lower-cased extension, else a generic page. The
// TUI maps each key to a Nerd Font glyph and the panel to one of these drawings, so the two tables are kept by hand
// and a language with no drawing of its own shares `code`. Maps, not objects, so a file called `constructor`
// finds nothing.

export const GENERIC = "file";

const NAMES = new Map<string, string>([
  ["Cargo.toml", "rust"],
  ["Cargo.lock", "rust"],
  ["Dockerfile", "docker"],
  ["Makefile", "make"],
  ["LICENSE", "license"],
  [".gitignore", "git"],
  [".gitattributes", "git"],
]);

const group = (icon: string, extensions: string): [string, string][] => extensions.split(" ").map((e) => [e, icon]);

const EXTENSIONS = new Map<string, string>([
  ...group("rust", "rs"),
  ...group("toml", "toml"),
  ...group("lock", "lock"),
  ...group("markdown", "md"),
  ...group("json", "json jsonc"),
  ...group("yaml", "yaml yml"),
  ...group("shell", "sh bash zsh fish"),
  ...group("js", "js mjs cjs jsx"),
  ...group("ts", "ts tsx mts cts"),
  ...group("code", "py go c h cpp hpp java kt swift rb php lua nix sql vue svelte astro html css scss less"),
  ...group("image", "png jpg jpeg gif svg webp ico"),
  ...group("text", "txt"),
  ...group("diff", "patch diff"),
]);

// `name` is a file name without its directory.
export function fileIcon(name: string): string {
  const whole = NAMES.get(name);
  if (whole !== undefined) return whole;
  const dot = name.lastIndexOf(".");
  return dot < 0 ? GENERIC : (EXTENSIONS.get(name.slice(dot + 1).toLowerCase()) ?? GENERIC);
}

// Every drawing the map can name, for the test that the files exist.
export const ICON_NAMES: string[] = [...new Set([GENERIC, ...NAMES.values(), ...EXTENSIONS.values()])];
