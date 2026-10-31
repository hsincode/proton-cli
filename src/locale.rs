//! Global launch locale (LANG / LC_*) presets.
//!
//! Steam / Lutris / umu run Japanese games without installing host locales:
//! pressure-vessel generates missing POSIX locales inside the container when
//! `LANG` / `LC_*` are set. We only need to pass the right values at launch.
//!
//! # Extending
//!
//! 1. Add a [`Locale`] entry to [`LOCALES`].
//! 2. Optionally keep it out of [`selectable`] until the TUI should offer it.
//! 3. Config stores the stable [`Locale::id`] string (e.g. `"ja"`, `"en"`).

/// One supported launch locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// Stable id written to `config.toml` (`locale = "ja"`).
    pub id: &'static str,
    /// Label shown in the TUI.
    pub label: &'static str,
    /// POSIX locale name for `LANG` / `LC_ALL` / `LC_CTYPE`.
    pub posix: &'static str,
}

/// Full catalog of known locales (may grow beyond what the UI shows).
pub const LOCALES: &[Locale] = &[
    Locale {
        id: "ja",
        label: "日本語 (Japanese)",
        // Host need not have ja_JP; umu/Steam runtime generates it in-container.
        posix: "ja_JP.UTF-8",
    },
    Locale {
        id: "en",
        label: "English",
        posix: "en_US.UTF-8",
    },
];

/// Locales offered in first-run / settings UI.
///
/// Keep this a deliberate subset when catalog entries exist but are not ready
/// for the picker yet. Today the UI only has Japanese and English.
pub fn selectable() -> &'static [Locale] {
    // When adding a third catalog entry without shipping UI yet, return a
    // slice of the first N entries (or a dedicated const list) instead of
    // `LOCALES` wholesale.
    LOCALES
}

/// Look up a catalog entry by config id.
pub fn find(id: &str) -> Option<&'static Locale> {
    LOCALES.iter().find(|l| l.id == id)
}

/// Env pairs applied at launch for this locale.
///
/// App-specific `env` in `apps.toml` overrides the same keys after these.
pub fn env_pairs(locale: &Locale) -> Vec<(String, String)> {
    let p = locale.posix.to_string();
    vec![
        ("LANG".into(), p.clone()),
        ("LC_ALL".into(), p.clone()),
        ("LC_CTYPE".into(), p),
    ]
}

/// Resolve env pairs from a config locale id (`None` / unknown → empty).
pub fn env_for_id(id: Option<&str>) -> Vec<(String, String)> {
    id.and_then(find).map(env_pairs).unwrap_or_default()
}

/// Index of `id` in [`selectable`], or `0` if missing.
pub fn selectable_index(id: Option<&str>) -> usize {
    let list = selectable();
    id.and_then(|want| list.iter().position(|l| l.id == want))
        .unwrap_or(0)
}

/// Effective locale id for a launch: per-app override, else global config.
pub fn effective_id<'a>(app_locale: Option<&'a str>, global: Option<&'a str>) -> Option<&'a str> {
    app_locale.or(global)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for l in LOCALES {
            assert!(seen.insert(l.id), "duplicate locale id {}", l.id);
        }
    }

    #[test]
    fn selectable_is_subset_of_catalog() {
        for l in selectable() {
            assert!(find(l.id).is_some());
        }
        // UI currently exposes exactly the two shipped locales.
        assert_eq!(selectable().len(), 2);
        assert_eq!(selectable()[0].id, "ja");
        assert_eq!(selectable()[1].id, "en");
    }

    #[test]
    fn env_pairs_set_lang_family() {
        let ja = find("ja").unwrap();
        let env = env_pairs(ja);
        assert!(env.iter().any(|(k, v)| k == "LANG" && v == "ja_JP.UTF-8"));
        assert!(env.iter().any(|(k, v)| k == "LC_ALL" && v == "ja_JP.UTF-8"));
        assert!(env
            .iter()
            .any(|(k, v)| k == "LC_CTYPE" && v == "ja_JP.UTF-8"));
    }

    #[test]
    fn env_for_unknown_is_empty() {
        assert!(env_for_id(None).is_empty());
        assert!(env_for_id(Some("zz-unknown")).is_empty());
    }

    #[test]
    fn effective_prefers_app_over_global() {
        assert_eq!(effective_id(Some("en"), Some("ja")), Some("en"));
        assert_eq!(effective_id(None, Some("ja")), Some("ja"));
        assert_eq!(effective_id(None, None), None);
    }

}
