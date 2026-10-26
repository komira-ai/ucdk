// SPDX-License-Identifier: Apache-2.0
//! Selection: what a run is asked to do. Selectors name what to run, never what to skip, so an
//! action added later is not run by an old selective command. `action:x` runs the action
//! without the validations attached to it, except that a Publish action always runs its own
//! (they run in its preflight, and no selector stages or commits without that gate);
//! `validation:y` runs one validation alone against what is already there. Any selector makes a
//! run selective.

use std::collections::BTreeSet;
use std::fmt;

use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::{Reason, Stage};

/// One thing to run. The order is the bytewise order of the text of the selector.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Selector {
    Action(String),
    Validation(String),
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Selector::Action(name) => write!(f, "action:{name}"),
            Selector::Validation(name) => write!(f, "validation:{name}"),
        }
    }
}

/// What a validation is attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// An action of the stage, by name.
    Action(String),
    /// The stage itself: it runs after the last action.
    Stage,
}

/// What a run does with one part of a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Run the action; `validations` says whether the validations attached to it run with it.
    /// That is so in a full run, and in a selective one only for a Publish action.
    Action { action: String, validations: bool },
    /// Run one validation alone.
    Validation { holder: Holder, name: String },
}

/// Why a selection was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// The text is neither `action:<name>` nor `validation:<name>`.
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
                    "{text:?} is not a selector: use action:<name> or validation:<name>"
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
                Some(("action", name)) if !name.is_empty() => Ok(Selector::Action(name.to_owned())),
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

    /// What `stage` runs under this selection, in the order of the stage: each action, then the
    /// validations attached to it that were selected, and after the last action the stage's own
    /// validations. Every selector must match something in the stage.
    pub fn resolve(&self, stage: &Stage) -> Result<Vec<Item>, SelectionError> {
        let choices: Vec<Selector> = stage
            .actions
            .iter()
            .map(|a| Selector::Action(a.name.clone()))
            .chain(
                stage
                    .actions
                    .iter()
                    .flat_map(|a| &a.validations)
                    .chain(&stage.validations)
                    .map(|v| Selector::Validation(v.name.clone())),
            )
            .collect();
        if let Some(missing) = self.0.iter().find(|s| !choices.contains(s)) {
            return Err(SelectionError::NoMatch {
                selector: missing.clone(),
                choices,
            });
        }
        let selected = |s: Selector| self.0.contains(&s);
        let mut items = Vec::new();
        for action in &stage.actions {
            // A Publish action's validations run in its preflight, whatever was selected.
            let gated = matches!(action.kind, Some(ActionKind::Publish(_)));
            let runs = !self.is_selective() || selected(Selector::Action(action.name.clone()));
            let with = !self.is_selective() || gated;
            if runs {
                items.push(Item::Action {
                    action: action.name.clone(),
                    validations: with,
                });
            }
            if self.is_selective() && !(runs && gated) {
                items.extend(
                    action
                        .validations
                        .iter()
                        .filter(|v| selected(Selector::Validation(v.name.clone())))
                        .map(|v| Item::Validation {
                            holder: Holder::Action(action.name.clone()),
                            name: v.name.clone(),
                        }),
                );
            }
        }
        items.extend(
            stage
                .validations
                .iter()
                .filter(|v| !self.is_selective() || selected(Selector::Validation(v.name.clone())))
                .map(|v| Item::Validation {
                    holder: Holder::Stage,
                    name: v.name.clone(),
                }),
        );
        Ok(items)
    }
}
