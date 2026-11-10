// The icon for a file in the sidebar, as the name of a drawing in `icons/` (no folder, no `.svg`). The keys are
// the TUI's `icons.rs` keys: a whole file name first, then the lower-cased extension, else a generic page. The
// TUI maps each key to a Nerd Font glyph and the panel to one of these drawings, so the two tables are kept by hand.
// The drawings are Material Icon Theme's, in their own colours (see icons/LICENSE-material-icon-theme), and keep its
// file names. Maps, not objects, so a file called `constructor` finds nothing.

export const GENERIC = "file";
// The drawing before a folder heading in the sidebar.
export const FOLDER = "folder";

const NAMES = new Map<string, string>([
  ["Cargo.toml", "rust"],
  ["Cargo.lock", "rust"],
  ["Dockerfile", "docker"],
  ["Makefile", "makefile"],
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
  ...group("console", "sh bash zsh fish"),
  ...group("javascript", "js mjs cjs"),
  ...group("react", "jsx"),
  ...group("typescript", "ts mts cts"),
  ...group("react_ts", "tsx"),
  ...group("python", "py"),
  ...group("go", "go"),
  ...group("c", "c"),
  ...group("h", "h"),
  ...group("cpp", "cpp"),
  ...group("hpp", "hpp"),
  ...group("java", "java"),
  ...group("kotlin", "kt"),
  ...group("swift", "swift"),
  ...group("ruby", "rb"),
  ...group("php", "php"),
  ...group("lua", "lua"),
  ...group("nix", "nix"),
  ...group("database", "sql"),
  ...group("vue", "vue"),
  ...group("svelte", "svelte"),
  ...group("astro", "astro"),
  ...group("html", "html"),
  ...group("css", "css"),
  ...group("sass", "scss"),
  ...group("less", "less"),
  ...group("image", "png jpg jpeg gif webp ico"),
  ...group("svg", "svg"),
  ...group("document", "txt"),
  ...group("diff", "patch diff"),
]);

// `name` is a file name without its directory.
export function fileIcon(name: string): string {
  const whole = NAMES.get(name);
  if (whole !== undefined) {
    return whole;
  }
  const dot = name.lastIndexOf(".");
  return dot < 0 ? GENERIC : (EXTENSIONS.get(name.slice(dot + 1).toLowerCase()) ?? GENERIC);
}

// Every drawing the map can name, for the test that the files exist.
export const ICON_NAMES: string[] = [...new Set([GENERIC, FOLDER, ...NAMES.values(), ...EXTENSIONS.values()])];

// The drawings for the panel's buttons, not for file types: they stay out of the maps above. The test checks each is a file in `icons/`.
export const UI_ICONS: string[] = ["panel-left", "panel-right", "swap", "columns", "refresh", "arrow-up", "arrow-down", "circle-check", "comment-plus", "send", "archive", "reply", "check", "undo", "pencil", "trash"];
