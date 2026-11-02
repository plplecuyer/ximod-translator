//! The locking rules of the scripts' strings (C-08, C-06): since B-18 the
//! script-scope rules of the exclusions file ([`crate::exclusions`],
//! decision D-73), seen as before by the readers of the scripts:
//!
//! * the expressions — a string one matches is locked with the provenance
//!   `lock:<id>` (« verrouillée : clé $ »…); tried in order, the first that
//!   matches names the rule;
//! * the functions whose string arguments are identifiers (read by C-06,
//!   `*` ending a name matches any continuation).
//!
//! The former `papyrus_rules.json` is gone; a user's copy of it is merged
//! once into the user's exclusions ([`crate::exclusions::Dirs::migrate_papyrus`]).

use regex::Regex;

use crate::exclusions::{Kind, RulesFile, Scope};

/// The former file name, in the user's `data/scripts/` (migration only).
pub const FILE: &str = "papyrus_rules.json";

/// The rules, compiled.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    patterns: Vec<(String, Regex, Option<Regex>)>,
    functions: Vec<String>,
}

impl Rules {
    /// The enabled script-scope rules of an exclusions file.
    pub fn from_exclusions(f: &RulesFile) -> Rules {
        let mut patterns = Vec::new();
        let mut functions = f.functions.clone();
        for r in f.rules.iter().filter(|r| r.enabled) {
            match r.kind {
                Kind::Regex if r.scope.contains(&Scope::Script) => {
                    let Ok(re) = Regex::new(&r.pattern) else {
                        continue;
                    };
                    let also = r.also.as_deref().and_then(|a| Regex::new(a).ok());
                    patterns.push((r.id.clone(), re, also));
                }
                Kind::Function => functions.push(r.pattern.clone()),
                _ => {}
            }
        }
        Rules {
            patterns,
            functions,
        }
    }

    /// The embedded copy of the shipped file.
    pub fn builtin() -> Rules {
        Rules::from_exclusions(&RulesFile::embedded())
    }

    /// The rules of the installation: the user's copy, else the shipped
    /// file.
    pub fn installed() -> Rules {
        Rules::from_exclusions(&crate::exclusions::Dirs::installed().load().0)
    }

    /// The id of the first pattern that matches `text`, if any.
    pub fn lock_of(&self, text: &str) -> Option<&str> {
        self.patterns
            .iter()
            .find(|(_, re, also)| {
                re.is_match(text) && also.as_ref().is_none_or(|a| a.is_match(text))
            })
            .map(|(id, _, _)| id.as_str())
    }

    /// Whether the string arguments of `function` (`Debug.Trace`) are
    /// identifiers.
    pub fn locks_function(&self, function: &str) -> bool {
        self.functions.iter().any(|f| match f.strip_suffix('*') {
            Some(prefix) => function.starts_with(prefix),
            None => f.eq_ignore_ascii_case(function),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_rules() {
        let r = Rules::builtin();
        assert_eq!(r.lock_of("$SKI_Pages"), Some("key"));
        assert_eq!(r.lock_of("Data/Interface/menu.swf"), Some("path"));
        assert_eq!(r.lock_of("DLCRobot.esm"), Some("path"));
        assert_eq!(r.lock_of("_root.Menu_mc"), Some("path"));
        assert_eq!(r.lock_of("1.2.3"), Some("noletter"));
        assert_eq!(r.lock_of("weaponSwing"), Some("word"));
        assert_eq!(r.lock_of("Raiders"), Some("word"));
        assert_eq!(r.lock_of("Skip the introduction?"), None);
        assert_eq!(r.lock_of("Prisoner's Inventory"), None);
        assert!(r.locks_function("Debug.Trace"));
        assert!(r.locks_function("StorageUtil.SetIntValue"));
        assert!(!r.locks_function("Debug.Notification"));
    }
}
