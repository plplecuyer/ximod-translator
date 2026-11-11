//! The arguments of `ximod-cli` (D-06, R-D06-01 / 02): the common options,
//! the command, its files and its options — parsed by hand against the
//! table of each command, an unknown option refused.

use std::collections::BTreeMap;

/// What an option takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takes {
    Flag,
    Value,
    /// A value, the option given several times.
    Values,
}

/// One option of a command.
#[derive(Debug, Clone, Copy)]
pub struct Opt {
    pub name: &'static str,
    pub takes: Takes,
}

const fn flag(name: &'static str) -> Opt {
    Opt {
        name,
        takes: Takes::Flag,
    }
}

const fn value(name: &'static str) -> Opt {
    Opt {
        name,
        takes: Takes::Value,
    }
}

const fn values(name: &'static str) -> Opt {
    Opt {
        name,
        takes: Takes::Values,
    }
}

/// A command: its name, how many files it takes (min, max) and its options.
pub struct Command {
    pub name: &'static str,
    pub files: (usize, usize),
    pub opts: &'static [Opt],
}

/// The options common to every command.
pub const COMMON: &[Opt] = &[
    value("game"),
    value("from"),
    value("to"),
    flag("json"),
    flag("quiet"),
    flag("verbose"),
    value("log"),
    value("lang"),
    flag("no-color"),
    value("config"),
    flag("help"),
];

pub const COMMANDS: &[Command] = &[
    Command {
        name: "info",
        files: (1, 1),
        opts: &[],
    },
    Command {
        name: "import",
        files: (1, 1),
        opts: &[value("format"), value("o")],
    },
    Command {
        name: "apply",
        files: (1, 1),
        opts: &[
            values("dict"),
            values("base"),
            flag("no-personal"),
            value("mode"),
            value("overwrite"),
            flag("write-dict"),
        ],
    },
    Command {
        name: "assist",
        files: (1, 1),
        opts: &[
            flag("glossary"),
            flag("patterns"),
            flag("derive"),
            value("overwrite"),
            flag("write-dict"),
        ],
    },
    Command {
        name: "translate",
        files: (1, 1),
        opts: &[
            value("service"),
            value("scope"),
            value("max-chars"),
            flag("write-dict"),
        ],
    },
    Command {
        name: "check",
        files: (1, 1),
        opts: &[value("fail-on"), value("report")],
    },
    Command {
        name: "export",
        files: (1, 1),
        opts: &[
            value("output"),
            value("statuses"),
            value("o"),
            value("dict"),
            flag("strict"),
            flag("no-esl"),
        ],
    },
    Command {
        name: "manifest",
        files: (1, 1),
        opts: &[values("input"), value("o")],
    },
    Command {
        name: "compare",
        files: (2, 2),
        opts: &[
            value("mode"),
            flag("fold"),
            value("report"),
            value("merge"),
            value("prefer"),
        ],
    },
    Command {
        name: "scan",
        files: (1, 1),
        opts: &[flag("no-subdirs"), flag("scripts"), value("report")],
    },
    Command {
        name: "convert",
        files: (1, 1),
        opts: &[value("o")],
    },
    Command {
        name: "package",
        files: (1, 1),
        opts: &[
            value("name"),
            value("version"),
            value("author"),
            value("website"),
            value("description"),
            value("plugin"),
            flag("no-info"),
            value("o"),
        ],
    },
    Command {
        name: "batch",
        files: (1, 1),
        opts: &[],
    },
];

pub fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// The values given to the options (a flag has an empty value).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Values(pub BTreeMap<String, Vec<String>>);

impl Values {
    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(|v| v.last()).map(String::as_str)
    }

    pub fn all(&self, name: &str) -> Vec<String> {
        self.0.get(name).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, name: &str, value: &str) {
        self.0
            .entry(name.to_string())
            .or_default()
            .push(value.to_string());
    }
}

/// The parsed command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub common: Values,
    pub command: Option<String>,
    pub files: Vec<String>,
    pub opts: Values,
    /// `--version` without a command.
    pub version: bool,
}

/// Why the line was refused: the argument at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    /// An option no table knows (`ximod-cli --help`).
    Unknown(String),
    /// An option given without its value.
    NoValue(String),
    /// A command no table knows.
    Command(String),
}

fn option_name(arg: &str) -> Option<(&str, Option<&str>)> {
    let body = if let Some(b) = arg.strip_prefix("--") {
        b
    } else {
        // `-o`, `-h`, `-q`, `-v`: one letter.
        let b = arg.strip_prefix('-')?;
        if b.chars().count() != 1 {
            return None;
        }
        b
    };
    if body.is_empty() {
        return None;
    }
    Some(match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    })
}

/// The long name of a one-letter option.
fn long_of(name: &str) -> &str {
    match name {
        "h" => "help",
        "q" => "quiet",
        "v" => "verbose",
        "out" => "o",
        other => other,
    }
}

/// Parse the arguments (the program name left out).
pub fn parse(args: &[String]) -> Result<Parsed, ArgError> {
    let mut p = Parsed::default();
    let mut cmd: Option<&'static Command> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        let Some((name, inline)) = option_name(a) else {
            if cmd.is_none() {
                cmd = Some(command(a).ok_or_else(|| ArgError::Command(a.clone()))?);
                p.command = Some(a.clone());
            } else {
                p.files.push(a.clone());
            }
            continue;
        };
        let name = long_of(name);
        if name == "version" && cmd.is_none() {
            p.version = true;
            continue;
        }
        let own = cmd.and_then(|c| c.opts.iter().find(|o| o.name == name));
        let (opt, into) = match (own, COMMON.iter().find(|o| o.name == name)) {
            (Some(o), _) => (o, &mut p.opts),
            (None, Some(o)) => (o, &mut p.common),
            (None, None) => return Err(ArgError::Unknown(a.clone())),
        };
        match opt.takes {
            Takes::Flag => {
                if inline.is_some() {
                    return Err(ArgError::Unknown(a.clone()));
                }
                into.set(name, "");
            }
            Takes::Value | Takes::Values => {
                let v = match inline {
                    Some(v) => v.to_string(),
                    None => {
                        let Some(v) = args.get(i) else {
                            return Err(ArgError::NoValue(a.clone()));
                        };
                        i += 1;
                        v.clone()
                    }
                };
                if opt.takes == Takes::Value {
                    into.0.remove(name);
                }
                into.set(name, &v);
            }
        }
    }
    Ok(p)
}

/// The common options of a line refused (language, configuration,
/// JSON): what can be read of it, the rest ignored.
pub fn parse_common_only(args: &[String]) -> Parsed {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        let Some((name, inline)) = option_name(a) else {
            continue;
        };
        let name = long_of(name);
        let Some(o) = COMMON.iter().find(|o| o.name == name) else {
            continue;
        };
        match (o.takes, inline) {
            (Takes::Flag, _) => p.common.set(name, ""),
            (_, Some(v)) => p.common.set(name, v),
            (_, None) => {
                if let Some(v) = args.get(i) {
                    i += 1;
                    p.common.set(name, v);
                }
            }
        }
    }
    p
}

/// The options a batch step may carry (the command's own, without `-o`
/// where the batch decides the place), by command name.
pub fn step_options(name: &str) -> Option<Vec<&'static str>> {
    let c = command(name)?;
    Some(
        c.opts
            .iter()
            .map(|o| o.name)
            .filter(|n| *n != "o")
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn common_and_own_options() {
        let p = parse(&v(&[
            "--json",
            "apply",
            "mod.esp",
            "--dict",
            "a.ximdb",
            "--dict=b.ximdb",
            "--mode",
            "relax",
            "--to",
            "fra",
            "--write-dict",
        ]))
        .unwrap();
        assert_eq!(p.command.as_deref(), Some("apply"));
        assert_eq!(p.files, vec!["mod.esp"]);
        assert!(p.common.has("json"));
        assert_eq!(p.common.get("to"), Some("fra"));
        assert_eq!(p.opts.all("dict"), vec!["a.ximdb", "b.ximdb"]);
        assert_eq!(p.opts.get("mode"), Some("relax"));
        assert!(p.opts.has("write-dict"));
    }

    #[test]
    fn refusals() {
        assert_eq!(
            parse(&v(&["apply", "x.esp", "--bogus"])),
            Err(ArgError::Unknown("--bogus".into()))
        );
        assert_eq!(
            parse(&v(&["frobnicate"])),
            Err(ArgError::Command("frobnicate".into()))
        );
        assert_eq!(
            parse(&v(&["export", "x.esp", "-o"])),
            Err(ArgError::NoValue("-o".into()))
        );
        // An option of another command.
        assert!(parse(&v(&["info", "x.esp", "--write-dict"])).is_err());
    }

    #[test]
    fn version_is_the_package_option_after_package() {
        let p = parse(&v(&["--version"])).unwrap();
        assert!(p.version && p.command.is_none());
        let p = parse(&v(&["package", "out", "--version", "1.2", "-o", "x.zip"])).unwrap();
        assert!(!p.version);
        assert_eq!(p.opts.get("version"), Some("1.2"));
        assert_eq!(p.opts.get("o"), Some("x.zip"));
    }
}
