//! Typed, tracked reads of one TDF section.
//!
//! [`Fields`] wraps a [`Section`] and remembers which keys a definition
//! consumed. Whatever is left over is returned by [`Fields::finish`] as the
//! def's `extra` map, so no key is lost, and is tallied in [`Coverage`].
//! Bad values never abort a load: they become [`Issue`]s.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tak_formats::tdf::{Decimal, Section};
use tak_math::Fx;

/// A value that could not be read as the type its key needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// VFS path of the file.
    pub file: String,
    /// Section path inside the file (`UNITINFO`, `WEAPON1/DAMAGE`).
    pub section: String,
    pub key: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [{}] {}: {}",
            self.file, self.section, self.key, self.message
        )
    }
}

/// Key usage for one section path, across every file loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SectionCoverage {
    /// Sections seen with this path.
    pub sections: u32,
    /// Keys a def field reads, with how many sections used them.
    pub mapped: BTreeMap<String, u32>,
    /// Keys kept only in `extra`, with how many sections used them.
    pub unmapped: BTreeMap<String, u32>,
}

/// Mapped vs unmapped keys per section path (`fbi:UNITINFO`, `fbi:WEAPON#`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub sections: BTreeMap<String, SectionCoverage>,
}

impl Coverage {
    pub fn merge(&mut self, other: &Coverage) {
        for (path, c) in &other.sections {
            let me = self.sections.entry(path.clone()).or_default();
            me.sections += c.sections;
            for (k, n) in &c.mapped {
                *me.mapped.entry(k.clone()).or_default() += n;
            }
            for (k, n) in &c.unmapped {
                *me.unmapped.entry(k.clone()).or_default() += n;
            }
        }
    }
}

/// Where issues and coverage go while loading.
#[derive(Debug, Default)]
pub struct Ctx {
    pub issues: Vec<Issue>,
    pub coverage: Coverage,
}

/// Collapses digit runs so `WEAPON1` and `WEAPON2` share a coverage row.
pub(crate) fn path_shape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_digits = false;
    for c in path.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            out.push(c.to_ascii_uppercase());
        }
    }
    out
}

/// 16.16 fixed point in an `i64`, for decimal keys whose values exceed the
/// [`Fx`] range (`buildtime` reaches 100000). Serializes as the raw `i64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(transparent)]
pub struct WideFx(pub i64);

impl WideFx {
    pub fn from_int(n: i64) -> WideFx {
        WideFx(n.saturating_mul(1 << Fx::FRAC_BITS))
    }

    /// Integer part, rounded toward negative infinity.
    pub fn to_int_floor(self) -> i64 {
        self.0 >> Fx::FRAC_BITS
    }

    /// As an [`Fx`], `None` when out of range.
    pub fn to_fx(self) -> Option<Fx> {
        i32::try_from(self.0).ok().map(Fx::from_raw)
    }
}

/// A decimal as 16.16 fixed point, `None` if it does not fit an [`Fx`].
pub fn decimal_to_fx(d: Decimal) -> Option<Fx> {
    let raw = d.to_fixed(Fx::FRAC_BITS)?;
    i32::try_from(raw).ok().map(Fx::from_raw)
}

/// Parses a value as [`Fx`] without floats (`1.25`, `-3`, `.5`).
pub fn parse_fx(s: &str) -> Option<Fx> {
    decimal_to_fx(Decimal::parse(s)?)
}

pub(crate) struct Fields<'s, 'c> {
    sec: &'s Section,
    file: &'s str,
    /// Section path for issues (`WEAPON1/DAMAGE`).
    path: String,
    /// Coverage row (`fbi:WEAPON#/DAMAGE`).
    row: String,
    used: BTreeSet<String>,
    ctx: &'c mut Ctx,
}

impl<'s, 'c> Fields<'s, 'c> {
    /// `kind` prefixes the coverage row (`fbi`, `feature`, `ota`).
    pub fn new(
        sec: &'s Section,
        file: &'s str,
        kind: &str,
        path: String,
        ctx: &'c mut Ctx,
    ) -> Self {
        let row = format!("{kind}:{}", path_shape(&path));
        Fields {
            sec,
            file,
            path,
            row,
            used: BTreeSet::new(),
            ctx,
        }
    }

    /// Like [`Fields::new`] but with an explicit coverage row, for files
    /// whose section names are data (feature names).
    pub fn with_row(
        sec: &'s Section,
        file: &'s str,
        row: String,
        path: String,
        ctx: &'c mut Ctx,
    ) -> Self {
        Fields {
            sec,
            file,
            path,
            row,
            used: BTreeSet::new(),
            ctx,
        }
    }

    pub fn ctx(&mut self) -> &mut Ctx {
        self.ctx
    }

    pub fn issue(&mut self, key: &str, message: String) {
        self.ctx.issues.push(Issue {
            file: self.file.to_string(),
            section: self.path.clone(),
            key: key.to_string(),
            message,
        });
    }

    /// The raw value, marking the key used. Empty values read as absent.
    pub fn raw(&mut self, key: &str) -> Option<&'s str> {
        self.used.insert(key.to_ascii_lowercase());
        let v = self.sec.get(key)?.trim();
        (!v.is_empty()).then_some(v)
    }

    pub fn string(&mut self, key: &str) -> Option<String> {
        self.raw(key).map(str::to_string)
    }

    /// Whitespace-separated words (`category = ARA BALLISTIC ATTACK`).
    pub fn words(&mut self, key: &str) -> Vec<String> {
        self.raw(key)
            .map(|v| v.split_ascii_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// A whole number that must fit `T`.
    pub fn int<T: TryFrom<i64>>(&mut self, key: &str) -> Option<T> {
        let v = self.raw(key)?;
        match v.parse::<i64>() {
            Ok(n) => match T::try_from(n) {
                Ok(t) => Some(t),
                Err(_) => {
                    self.issue(key, format!("{v:?} is out of range"));
                    None
                }
            },
            Err(_) => {
                self.issue(key, format!("{v:?} is not an integer"));
                None
            }
        }
    }

    /// A number read as an exact decimal and converted to 16.16 [`Fx`].
    pub fn fx(&mut self, key: &str) -> Option<Fx> {
        let v = self.raw(key)?;
        match Decimal::parse(v) {
            Some(d) => match decimal_to_fx(d) {
                Some(f) => Some(f),
                None => {
                    self.issue(key, format!("{v:?} does not fit 16.16 fixed point"));
                    None
                }
            },
            None => {
                self.issue(key, format!("{v:?} is not a number"));
                None
            }
        }
    }

    /// A number as 16.16 fixed point in an `i64` ([`WideFx`]).
    pub fn wide(&mut self, key: &str) -> Option<WideFx> {
        let v = self.raw(key)?;
        match Decimal::parse(v).and_then(|d| d.to_fixed(Fx::FRAC_BITS)) {
            Some(raw) => Some(WideFx(raw)),
            None => {
                self.issue(key, format!("{v:?} is not a number"));
                None
            }
        }
    }

    /// `0` false, other integers true, absent false.
    pub fn flag(&mut self, key: &str) -> bool {
        self.int::<i64>(key).is_some_and(|n| n != 0)
    }

    /// Marks keys as mapped without reading them (consumed elsewhere).
    pub fn consume(&mut self, keys: &[&str]) {
        for k in keys {
            self.used.insert(k.to_ascii_lowercase());
        }
    }

    /// Records coverage and returns the entries no field read: lowercased
    /// key -> last value (the last duplicate wins, as in [`Section::get`]).
    pub fn finish(self) -> BTreeMap<String, String> {
        let cov = self.ctx.coverage.sections.entry(self.row).or_default();
        cov.sections += 1;
        let mut extra = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for e in &self.sec.entries {
            let k = e.key.to_ascii_lowercase();
            let first = seen.insert(k.clone());
            if self.used.contains(&k) {
                if first {
                    *cov.mapped.entry(k).or_default() += 1;
                }
            } else {
                if first {
                    *cov.unmapped.entry(k.clone()).or_default() += 1;
                }
                extra.insert(k, e.value.trim().to_string());
            }
        }
        extra
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_formats::tdf;

    fn doc(s: &str) -> tdf::Document {
        tdf::parse(s.as_bytes()).unwrap()
    }

    #[test]
    fn reads_and_tracks() {
        let d = doc(
            "[A]\n{\n\tx = 1.25;\n\tn = 7;\n\tw = a b  c;\n\tunused = q;\n\te = ;\n\tbad = 1.5;\n}\n",
        );
        let mut ctx = Ctx::default();
        let mut f = Fields::new(&d.sections[0], "f.tdf", "t", "A".into(), &mut ctx);
        assert_eq!(f.fx("X"), Some(Fx::from_ratio(5, 4).unwrap()));
        assert_eq!(f.int::<u8>("n"), Some(7));
        assert_eq!(f.words("w"), vec!["a", "b", "c"]);
        assert_eq!(f.string("e"), None);
        assert_eq!(f.int::<i32>("bad"), None);
        assert_eq!(f.int::<i32>("missing"), None);
        let extra = f.finish();
        assert_eq!(extra.len(), 1);
        assert_eq!(extra["unused"], "q");
        assert_eq!(ctx.issues.len(), 1);
        assert_eq!(ctx.issues[0].key, "bad");
        let cov = &ctx.coverage.sections["t:A"];
        assert_eq!(cov.unmapped["unused"], 1);
        assert_eq!(cov.mapped["x"], 1);
        assert!(!cov.mapped.contains_key("missing"));
    }

    #[test]
    fn out_of_range() {
        let d = doc("[A]\n{\n\tn = 300;\n\tf = 40000;\n}\n");
        let mut ctx = Ctx::default();
        let mut f = Fields::new(&d.sections[0], "f", "t", "A".into(), &mut ctx);
        assert_eq!(f.int::<u8>("n"), None);
        assert_eq!(f.fx("f"), None);
        assert_eq!(f.wide("f"), Some(WideFx::from_int(40000)));
        assert_eq!(WideFx::from_int(40000).to_int_floor(), 40000);
        assert_eq!(WideFx::from_int(40000).to_fx(), None);
        f.finish();
        assert_eq!(ctx.issues.len(), 2);
    }

    #[test]
    fn shapes() {
        assert_eq!(path_shape("weapon12/damage"), "WEAPON#/DAMAGE");
        assert_eq!(parse_fx("-.5"), Some(Fx::from_ratio(-1, 2).unwrap()));
        assert_eq!(parse_fx("x"), None);
    }
}
