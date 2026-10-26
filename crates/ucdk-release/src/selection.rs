// SPDX-License-Identifier: Apache-2.0
//! Selection: what a run is asked to do. Selectors name what to run, never what to skip, so a
//! step added later is not run by an old selective command. `step:x` runs the step without its
//! validations; `validation:y` runs one validation alone against what is already there. Any
//! selector makes a run selective.

use std::collections::BTreeSet;
use std::fmt;

use ucdk_release_proto::v1::{Reason, Stage};

/// One thing to run. The order is the bytewise order of the text of the selector.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Selector {
    Step(String),
    Validation(String),
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Step(name) => write!(f, "step:{name}"),
            Selector::Validation(name) => write!(f, "validation:{name}"),
        }
    }
}

/// What a run does with one part of a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Run the step; with its validations in a full run, without them when only the step was
    /// selected.
    Step { step: String, validations: bool },
    /// Run one validation of a step alone.
    Validation { step: String, name: String },
}

/// Why a selection was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// The text is neither `step:<name>` nor `validation:<name>`.
    Malformed(String),
    /// The selector matches nothing in the stage; the choices are what the stage holds.
    NoMatch {
        selector: Selector,
        choices: Vec<Selector>,
    },
}

impl SelectionError {
    pub fn reason(&self) -> Reason {
        match self {
            SelectionError::Malformed(_) => Reason::Selector,
            SelectionError::NoMatch { .. } => Reason::SelectorNoMatch,
        }
    }
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionError::Malformed(text) => {
                write!(
                    f,
                    "{text:?} is not a selector: use step:<name> or validation:<name>"
                )
            }
            SelectionError::NoMatch { selector, choices } => {
                let all: Vec<String> = choices.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "{selector} matches nothing in the stage; the choices are {}",
                    all.join(", ")
                )
            }
        }
    }
}

/// The selectors of a run. Repeating a selector selects it once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection(BTreeSet<Selector>);

impl Selection {
    /// Reads the texts of the `--only` flags.
    pub fn parse(texts: &[String]) -> Result<Self, SelectionError> {
        texts
            .iter()
            .map(|text| match text.split_once(':') {
                Some(("step", name)) if !name.is_empty() => Ok(Selector::Step(name.to_owned())),
                Some(("validation", name)) if !name.is_empty() => {
                    Ok(Selector::Validation(name.to_owned()))
                }
                _ => Err(SelectionError::Malformed(text.clone())),
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    /// Whether any selector was given: a selective run is never reported as a full one.
    pub fn is_selective(&self) -> bool {
        !self.0.is_empty()
    }

    /// The selectors as text, sorted bytewise: what the run hash covers.
    pub fn texts(&self) -> Vec<String> {
        self.0.iter().map(ToString::to_string).collect()
    }

    /// What `stage` runs under this selection, in the order of the stage. Every selector must
    /// match something in the stage.
    pub fn resolve(&self, stage: &Stage) -> Result<Vec<Item>, SelectionError> {
        let choices: Vec<Selector> = stage
            .steps
            .iter()
            .map(|s| Selector::Step(s.name.clone()))
            .chain(
                stage
                    .steps
                    .iter()
                    .flat_map(|s| &s.validations)
                    .map(|v| Selector::Validation(v.name.clone())),
            )
            .collect();
        if let Some(missing) = self.0.iter().find(|s| !choices.contains(s)) {
            return Err(SelectionError::NoMatch {
                selector: missing.clone(),
                choices,
            });
        }
        let mut items = Vec::new();
        for step in &stage.steps {
            if !self.is_selective() {
                items.push(Item::Step {
                    step: step.name.clone(),
                    validations: true,
                });
                continue;
            }
            if self.0.contains(&Selector::Step(step.name.clone())) {
                items.push(Item::Step {
                    step: step.name.clone(),
                    validations: false,
                });
            }
            for v in &step.validations {
                if self.0.contains(&Selector::Validation(v.name.clone())) {
                    items.push(Item::Validation {
                        step: step.name.clone(),
                        name: v.name.clone(),
                    });
                }
            }
        }
        Ok(items)
    }
}
