//! A small command line parser: positional words, `--name value` (or `--name=value`) options
//! and `--name` switches. Each command names the options it takes; any other is refused.

use std::collections::{BTreeMap, BTreeSet};

use crate::{usage, Result};

#[derive(Debug, Default)]
pub(crate) struct Args {
    positional: Vec<String>,
    values: BTreeMap<&'static str, String>,
    lists: BTreeMap<&'static str, Vec<String>>,
    switches: BTreeSet<&'static str>,
}

impl Args {
    /// Parses a command's arguments. `values` are the options that take a value, `switches`
    /// the ones that don't. An option given twice, or one the command doesn't take, is refused.
    pub(crate) fn parse(
        args: &[String],
        values: &[&'static str],
        switches: &[&'static str],
    ) -> Result<Self> {
        Self::parse_repeating(args, values, &[], switches)
    }

    /// Like `parse`, with `repeating` options that take a value and may be given any number
    /// of times; `all` reads them back in the order given.
    pub(crate) fn parse_repeating(
        args: &[String],
        values: &[&'static str],
        repeating: &[&'static str],
        switches: &[&'static str],
    ) -> Result<Self> {
        let mut parsed = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let Some(option) = arg.strip_prefix("--") else {
                if arg.len() > 1 && arg.starts_with('-') {
                    return Err(usage(format!("unknown option '{arg}'")));
                }
                parsed.positional.push(arg.clone());
                continue;
            };
            let (name, inline) = match option.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (option, None),
            };
            if let Some(&name) = switches.iter().find(|s| **s == name) {
                if inline.is_some() {
                    return Err(usage(format!("--{name} takes no value")));
                }
                if !parsed.switches.insert(name) {
                    return Err(usage(format!("--{name} is given twice")));
                }
            } else if let Some(&name) = values.iter().chain(repeating).find(|s| **s == name) {
                let value = match inline {
                    Some(value) => value,
                    None => args
                        .next()
                        .filter(|v| !v.starts_with("--"))
                        .cloned()
                        .ok_or_else(|| usage(format!("--{name} needs a value")))?,
                };
                if repeating.contains(&name) {
                    parsed.lists.entry(name).or_default().push(value);
                } else if parsed.values.insert(name, value).is_some() {
                    return Err(usage(format!("--{name} is given twice")));
                }
            } else {
                return Err(usage(format!("unknown option '--{name}'")));
            }
        }
        Ok(parsed)
    }

    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Every value of a repeating option, in the order given.
    pub(crate) fn all(&self, name: &str) -> &[String] {
        self.lists.get(name).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn required(&self, name: &str) -> Result<&str> {
        self.value(name)
            .ok_or_else(|| usage(format!("--{name} is required")))
    }

    pub(crate) fn switch(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    /// The names of the value options given, in name order.
    pub(crate) fn given(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.values.keys().copied()
    }

    /// The one positional word, `what` naming it for the error.
    pub(crate) fn one(&self, what: &str) -> Result<&str> {
        match self.positional.as_slice() {
            [word] => Ok(word),
            [] => Err(usage(format!("give {what}"))),
            [_, extra, ..] => Err(usage(format!("unexpected argument '{extra}'"))),
        }
    }

    /// The positional words from the `from`th on (none when there are fewer).
    pub(crate) fn rest(&self, from: usize) -> &[String] {
        self.positional.get(from..).unwrap_or_default()
    }

    /// Exactly `N` positional words, `names` saying what each is for the error.
    pub(crate) fn words<const N: usize>(&self, names: [&str; N]) -> Result<[&str; N]> {
        if let Some(extra) = self.positional.get(N) {
            return Err(usage(format!("unexpected argument '{extra}'")));
        }
        let mut words = [""; N];
        for (i, name) in names.iter().enumerate() {
            words[i] = self
                .positional
                .get(i)
                .ok_or_else(|| usage(format!("give {name}")))?;
        }
        Ok(words)
    }

    /// Refuses any positional word, for commands that take none.
    pub(crate) fn none(&self) -> Result<()> {
        match self.positional.first() {
            None => Ok(()),
            Some(extra) => Err(usage(format!("unexpected argument '{extra}'"))),
        }
    }
}
