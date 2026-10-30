use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Required,
    Optional,
    Incompatible,
    Unordered,
}
#[derive(Clone, Copy)]
pub(super) enum Comparison {
    Equal,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}
#[derive(Clone, Copy)]
pub(super) struct Constraint {
    operator: Comparison,
    required: [u32; 3],
}
impl Constraint {
    pub(super) fn matches(self, actual: &str) -> bool {
        version(actual).is_ok_and(|actual| match self.operator {
            Comparison::Equal => actual == self.required,
            Comparison::Greater => actual > self.required,
            Comparison::GreaterEqual => actual >= self.required,
            Comparison::Less => actual < self.required,
            Comparison::LessEqual => actual <= self.required,
        })
    }
}
pub(super) struct Dependency<'a> {
    pub name: &'a str,
    pub kind: Kind,
    pub constraint: Option<Constraint>,
}
impl<'a> Dependency<'a> {
    pub(super) fn parse(text: &'a str) -> Result<Self> {
        let text = text.trim();
        let (kind, text) = if let Some(text) = text.strip_prefix("(?)") {
            (Kind::Optional, text.trim())
        } else if let Some(text) = text.strip_prefix(['?', '+']) {
            (Kind::Optional, text.trim())
        } else if let Some(text) = text.strip_prefix('!') {
            (Kind::Incompatible, text.trim())
        } else if let Some(text) = text.strip_prefix('~') {
            (Kind::Unordered, text.trim())
        } else {
            (Kind::Required, text)
        };
        let (name, comparison) = text.find(['<', '>', '=']).map_or((text, ""), |index| {
            (text[..index].trim(), text[index..].trim())
        });
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(Error::bad("Invalid mod dependency name"));
        }
        let constraint = if comparison.is_empty() {
            None
        } else {
            let (operator, length) = if comparison.starts_with(">=") {
                (Comparison::GreaterEqual, 2)
            } else if comparison.starts_with("<=") {
                (Comparison::LessEqual, 2)
            } else if comparison.starts_with('=') {
                (Comparison::Equal, 1)
            } else if comparison.starts_with('>') {
                (Comparison::Greater, 1)
            } else {
                (Comparison::Less, 1)
            };
            Some(Constraint {
                operator,
                required: dependency_version(comparison[length..].trim())?,
            })
        };
        Ok(Self {
            name,
            kind,
            constraint,
        })
    }
}
