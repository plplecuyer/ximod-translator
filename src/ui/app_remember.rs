//! « Se souvenir de mon choix » (E-03): the box of the eligible dialogs,
//! the remembered answers applied without asking, and the « Questions
//! mémorisées » group of Paramètres › Général.

use eframe::egui;

use super::XimodApp;
use crate::i18n::I18n;
use crate::journal::{Kind, Level};
use crate::remember::{Answer, Question, Scope};

/// The box of the dialog on screen: unchecked at each display (R-E03-03).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RememberDraft {
    question: Option<Question>,
    on: bool,
    forever: bool,
}

impl RememberDraft {
    /// The scope chosen for `q`, when its box is checked.
    fn scope_for(&self, q: Question) -> Option<Scope> {
        (self.question == Some(q) && self.on).then_some(if self.forever {
            Scope::Forever
        } else {
            Scope::Session
        })
    }
}

/// The « Se souvenir de mon choix » box of `q` and, once checked, its two
/// scopes (session by default, D-109).
pub(crate) fn remember_box(ui: &mut egui::Ui, i18n: &I18n, draft: &mut RememberDraft, q: Question) {
    if draft.question != Some(q) {
        *draft = RememberDraft {
            question: Some(q),
            ..RememberDraft::default()
        };
    }
    ui.checkbox(&mut draft.on, i18n.t("remember-choice"));
    if draft.on {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.radio_value(&mut draft.forever, false, i18n.t("remember-session"));
            ui.radio_value(&mut draft.forever, true, i18n.t("remember-forever"));
        });
    }
}

/// « <question> — <answer> » texts of a remembered answer.
fn texts(i18n: &I18n, q: Question, a: Answer) -> (String, String) {
    (
        i18n.t(q.i18n_key()),
        q.answer_key(a).map(|k| i18n.t(k)).unwrap_or_default(),
    )
}

impl XimodApp {
    /// The remembered answer of `q`, applied instead of the dialog: the
    /// journal notes it so that nothing is silent (R-E03-01).
    pub(crate) fn remembered(&mut self, q: Question) -> Option<Answer> {
        let a = self.remembered.get(q)?;
        let (question, answer) = texts(&self.i18n, q, a);
        let text = self.i18n.t_args(
            "remember-applied",
            &[("question", &question), ("answer", &answer)],
        );
        self.journal_add(Level::Info, Kind::Config, &text);
        Some(a)
    }

    /// The dialog of `q` closed: `Some(answer)` by a button that may be
    /// remembered, `None` by Annuler, Échap or the cross — nothing kept.
    pub(crate) fn remember_answered(&mut self, q: Question, answer: Option<Answer>) {
        let draft = std::mem::take(&mut self.remember_draft);
        let (Some(a), Some(scope)) = (answer, draft.scope_for(q)) else {
            return;
        };
        if self.remembered.remember(q, a, scope) {
            self.sync_confirmations();
        }
    }

    /// Config.ini follows the permanent answers.
    fn sync_confirmations(&mut self) {
        let map = self.remembered.to_config();
        if map != self.config.confirmations {
            self.config.confirmations = map;
            let _ = self.config.save();
        }
    }

    /// « Réafficher les questions »: everything forgotten, at once
    /// (R-E03-05).
    pub(crate) fn remember_reset(&mut self) {
        self.remembered.clear();
        self.sync_confirmations();
        let msg = self.i18n.t("settings-reask-done");
        self.journal_add(Level::Info, Kind::Config, &msg);
        self.status = msg;
    }

    /// Paramètres › Général › « Questions mémorisées ».
    pub(crate) fn settings_remembered(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(self.i18n.t("settings-remembered")).strong());
        let list = self.remembered.list();
        if list.is_empty() {
            ui.label(
                egui::RichText::new(self.i18n.t("settings-remembered-none"))
                    .weak()
                    .small(),
            );
        } else {
            let (session, forever) = self.remembered.counts();
            ui.label(self.i18n.t_args(
                "settings-remembered-n",
                &[
                    ("n", &list.len().to_string()),
                    ("session", &session.to_string()),
                    ("forever", &forever.to_string()),
                ],
            ));
            for (q, a, scope) in list {
                let (question, answer) = texts(&self.i18n, q, a);
                let line = self.i18n.t_args(
                    "settings-remembered-line",
                    &[
                        ("question", &question),
                        ("answer", &answer),
                        ("scope", &self.i18n.t(scope.i18n_key())),
                    ],
                );
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(format!("• {line}"));
                });
            }
        }
        ui.add_space(4.0);
        if ui
            .add_enabled(
                !self.remembered.is_empty(),
                egui::Button::new(self.i18n.t("settings-reask")),
            )
            .clicked()
        {
            self.remember_reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_is_kept_only_for_the_checked_question() {
        let mut d = RememberDraft::default();
        assert_eq!(d.scope_for(Question::Propagate), None);
        d.question = Some(Question::Propagate);
        assert_eq!(d.scope_for(Question::Propagate), None);
        d.on = true;
        assert_eq!(d.scope_for(Question::Propagate), Some(Scope::Session));
        assert_eq!(d.scope_for(Question::DictGame), None);
        d.forever = true;
        assert_eq!(d.scope_for(Question::Propagate), Some(Scope::Forever));
    }
}
