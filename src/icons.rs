//! The Nerd Font glyph for a file in the sidebar. Code points come from the `nvim-web-devicons`
//! table, kept to the private use area U+E000 to U+F8FF, where each is one `char` and one cell.

/// The glyph for a file with no entry: a plain page.
const GENERIC: char = '\u{f016}';

/// Whole file names. They win over the extension.
const NAMES: &[(&str, char)] = &[
    ("Cargo.toml", '\u{e68b}'),
    ("Cargo.lock", '\u{e68b}'),
    ("Dockerfile", '\u{e7b0}'),
    ("Makefile", '\u{e779}'),
    ("LICENSE", '\u{e60a}'),
    (".gitignore", '\u{e702}'),
    (".gitattributes", '\u{e702}'),
];

/// Extensions in lower case, without the dot.
const EXTENSIONS: &[(&str, char)] = &[
    ("rs", '\u{e68b}'),
    ("toml", '\u{e6b2}'),
    ("lock", '\u{e672}'),
    ("md", '\u{f48a}'),
    ("json", '\u{e60b}'),
    ("yaml", '\u{e8eb}'),
    ("yml", '\u{e8eb}'),
    ("sh", '\u{e795}'),
    ("bash", '\u{e760}'),
    ("zsh", '\u{e795}'),
    ("fish", '\u{e795}'),
    ("py", '\u{e606}'),
    ("js", '\u{e60c}'),
    ("mjs", '\u{e60c}'),
    ("cjs", '\u{e60c}'),
    ("ts", '\u{e628}'),
    ("tsx", '\u{e7ba}'),
    ("jsx", '\u{e625}'),
    ("mts", '\u{e628}'),
    ("cts", '\u{e628}'),
    ("vue", '\u{e6a0}'),
    ("svelte", '\u{e697}'),
    ("astro", '\u{e6b3}'),
    ("jsonc", '\u{e60b}'),
    ("less", '\u{e614}'),
    ("webp", '\u{e60d}'),
    ("ico", '\u{e60d}'),
    ("html", '\u{e736}'),
    ("css", '\u{e6b8}'),
    ("scss", '\u{e603}'),
    ("go", '\u{e627}'),
    ("c", '\u{e61e}'),
    ("h", '\u{f0fd}'),
    ("cpp", '\u{e61d}'),
    ("hpp", '\u{f0fd}'),
    ("java", '\u{e738}'),
    ("kt", '\u{e634}'),
    ("swift", '\u{e755}'),
    ("rb", '\u{e791}'),
    ("php", '\u{e608}'),
    ("lua", '\u{e620}'),
    ("nix", '\u{f313}'),
    ("sql", '\u{e706}'),
    ("txt", '\u{f15c}'),
    ("png", '\u{e60d}'),
    ("jpg", '\u{e60d}'),
    ("jpeg", '\u{e60d}'),
    ("gif", '\u{e60d}'),
    ("svg", '\u{e60d}'),
    ("patch", '\u{e728}'),
    ("diff", '\u{e728}'),
];

/// The glyph for `name`, a file name without its directory.
pub fn icon(name: &str) -> char {
    if let Some((_, glyph)) = NAMES.iter().find(|(whole, _)| *whole == name) {
        return *glyph;
    }
    let Some((_, extension)) = name.rsplit_once('.') else {
        return GENERIC;
    };
    let extension = extension.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(GENERIC, |(_, glyph)| *glyph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::width::char_width;

    #[test]
    fn a_whole_name_wins_over_its_extension() {
        assert_eq!(icon("Cargo.toml"), '\u{e68b}');
        assert_ne!(icon("Cargo.toml"), icon("other.toml"));
        assert_eq!(icon("other.toml"), '\u{e6b2}');
    }

    #[test]
    fn a_vue_file_has_its_own_glyph() {
        assert_eq!(icon("StatusTabs.vue"), '\u{e6a0}');
    }

    #[test]
    fn the_extension_ignores_case() {
        assert_eq!(icon("main.RS"), icon("main.rs"));
        assert_ne!(icon("main.RS"), GENERIC);
    }

    #[test]
    fn a_name_with_no_dot_or_an_unknown_extension_gets_the_generic_glyph() {
        assert_eq!(icon("README"), GENERIC);
        assert_eq!(icon("data.unknownext"), GENERIC);
        assert_eq!(icon(""), GENERIC);
    }

    #[test]
    fn every_glyph_is_one_cell_in_the_private_use_area() {
        let glyphs = NAMES.iter().chain(EXTENSIONS).map(|(_, glyph)| *glyph);
        for glyph in glyphs.chain([GENERIC]) {
            assert!(('\u{e000}'..='\u{f8ff}').contains(&glyph), "{glyph:?}");
            assert_eq!(char_width(glyph), 1, "{glyph:?}");
        }
    }

    #[test]
    fn every_extension_is_lower_case() {
        for (extension, _) in EXTENSIONS {
            assert_eq!(*extension, extension.to_ascii_lowercase());
        }
    }
}
