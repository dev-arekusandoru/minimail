use super::*;

/// The questions the page offers; `Expects reply` stays hidden.
const TAGGED: [QuestionKey; 4] = [
    QuestionKey::Spam,
    QuestionKey::NeedsReply,
    QuestionKey::Urgency,
    QuestionKey::Kind,
];

/// The muted line under a row: the tag the check applies.
fn tag_line(question: QuestionKey) -> &'static str {
    match question {
        QuestionKey::Spam => "Applies the Possible spam tag.",
        QuestionKey::NeedsReply => "Applies the Needs reply tag.",
        QuestionKey::Urgency => "Applies the Urgent tag with its level.",
        QuestionKey::Kind => "Applies the Kind tag (person, receipt, and the rest).",
        QuestionKey::Folder => "Suggests a folder to move the message to.",
        QuestionKey::ExpectsReply => "Applies the Awaiting reply tag.",
    }
}

/// The judge policy's shipped mode for `question`.
fn default_mode(question: QuestionKey) -> Mode {
    JudgePolicy::default().mode(question)
}

/// `mode`'s segment in [`CLASSIFIER_MODES`], which lists Auto, Review, Off.
fn mode_index(mode: Mode) -> usize {
    match mode {
        Mode::Auto(_) => 0,
        Mode::Review => 1,
        Mode::Off => 2,
    }
}

/// `confidence`'s position in [`CONFIDENCES`], which lists High, Medium, Low like
/// [`Confidence::ALL`].
fn confidence_index(confidence: Confidence) -> usize {
    match confidence {
        Confidence::High => 0,
        Confidence::Medium => 1,
        Confidence::Low => 2,
    }
}

/// The stored key for `confidence`.
fn confidence_key(confidence: Confidence) -> &'static str {
    CONFIDENCES[confidence_index(confidence)].0
}

fn confidence_from_key(value: &str) -> Confidence {
    match value {
        "high" => Confidence::High,
        "low" => Confidence::Low,
        _ => Confidence::Medium,
    }
}

impl SettingsPanel {
    /// The "Jev connection" group: the API key (masked, never shown), the endpoint, the model,
    /// and the Save / Test connection actions that apply them.
    fn jev_group(&self, weak: &Weak) -> SettingGroup {
        let keywords = ["jev", "api key", "base url", "model", "classifier", "endpoint", "connection"];
        let key_input = self.jev_key_input.clone();
        let key_weak = weak.clone();
        let key_row = row(
            "API key",
            "Stored in the system keychain and never shown again.",
            &keywords,
            no_reset(weak),
            Rc::new(move |_, _, cx| {
                let has_key = key_weak.read_with(cx, |this, _| this.jev_has_key).unwrap_or(false);
                let remove = key_weak.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Input::new(&key_input).id("jev-key").small().w(px(120.)))
                    .when(has_key, |this| {
                        this.child(
                            Button::new("jev-remove-key")
                                .label("Remove key")
                                .small()
                                .ghost()
                                .on_click(move |_, _, cx| {
                                    remove.update(cx, |this, cx| this.remove_jev_key(cx)).ok();
                                }),
                        )
                    })
                    .into_any_element()
            }),
        );

        let base_input = self.jev_base_input.clone();
        let base_row = row(
            "Base URL",
            "The Jev endpoint every request goes to.",
            &keywords,
            no_reset(weak),
            Rc::new(move |_, _, _| {
                Input::new(&base_input).id("jev-base").small().w(px(170.)).into_any_element()
            }),
        );

        let model_input = self.jev_model_input.clone();
        let model_row = row(
            "Model",
            "The Jev model alias to send with each request.",
            &keywords,
            no_reset(weak),
            Rc::new(move |_, _, _| {
                Input::new(&model_input).id("jev-model").small().w(px(140.)).into_any_element()
            }),
        );

        let actions_weak = weak.clone();
        let actions_row = row(
            "Connection",
            "Save applies all three settings and starts classifying. Test connection sends one request and saves nothing.",
            &keywords,
            no_reset(weak),
            Rc::new(move |_, _, cx| {
                let (error, test) = actions_weak
                    .read_with(cx, |this, _| (this.jev_error.clone(), this.jev_test.clone()))
                    .unwrap_or((None, None));
                let (save, test_click) = (actions_weak.clone(), actions_weak.clone());
                let t = cx.theme();
                let message = jev_message(error, test, t);
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("jev-save").label("Save").small().on_click(move |_, _, cx| {
                                    save.update(cx, |this, cx| this.save_jev(cx)).ok();
                                }),
                            )
                            .child(
                                Button::new("jev-test")
                                    .label("Test connection")
                                    .small()
                                    .outline()
                                    .on_click(move |_, _, cx| {
                                        test_click.update(cx, |this, cx| this.test_jev(cx)).ok();
                                    }),
                            ),
                    )
                    .children(message)
                    .into_any_element()
            }),
        );

        SettingGroup::new()
            .title("Jev connection")
            .item(key_row)
            .item(base_row)
            .item(model_row)
            .item(actions_row)
    }

    pub(super) fn classifier_page(&self, weak: &Weak) -> SettingPage {
        let summaries = SettingGroup::new().item(row(
            "Thread summaries",
            "Write a short summary above each conversation.",
            &["summaries", "opt in to generated summaries"],
            switch_undo(weak, |this| this.summaries, false, SettingsPanel::set_summaries),
            switch(weak, |this| this.summaries, SettingsPanel::set_summaries),
        ));
        let mut tagging = SettingGroup::new().title("Tagging");
        for question in TAGGED {
            tagging = tagging.item(row(
                question.label(),
                tag_line(question),
                &["classifier", "auto review off", "tagging"],
                Undo::of_panel(
                    weak,
                    move |this| this.policy.mode(question) != default_mode(question),
                    move |this, cx| this.set_mode(question, default_mode(question), cx),
                ),
                segmented(
                    format!("ai-mode-group-{}", question.label()),
                    mode_entries(question),
                    weak,
                    move |this| mode_index(this.policy.mode(question)),
                    move |this, index, cx| {
                        this.set_classifier_mode(question, CLASSIFIER_MODES[index].0, cx)
                    },
                ),
            ));
            if matches!(self.policy.mode(question), Mode::Auto(_)) {
                let default = default_confidence(question);
                let (read, write) = (weak.clone(), weak.clone());
                tagging = tagging.item(row(
                    format!("{} confidence", question.label()),
                    "Apply without review once the classifier is this sure.",
                    &["classifier confidence", "high medium low"],
                    Undo::of_panel(
                        weak,
                        move |this| this.policy.mode(question) != Mode::Auto(default),
                        move |this, cx| this.set_confidence(question, default, cx),
                    ),
                    select(
                        options(&CONFIDENCES),
                        Rc::new({
                            let read = read.clone();
                            move |cx: &App| {
                                read.read_with(cx, |this, _| {
                                    confidence_of(this.policy.mode(question), default)
                                })
                                .unwrap_or_default()
                            }
                        }),
                        Rc::new(move |value: SharedString, cx: &mut App| {
                            write
                                .update(cx, |this, cx| {
                                    this.set_confidence(question, confidence_from_key(&value), cx)
                                })
                                .ok();
                        }),
                    ),
                ));
            }
        }
        // The Folder question is suggestion-only (never auto-applied): Off / Review only.
        tagging = tagging.item(row(
            QuestionKey::Folder.label(),
            "Suggests a folder to move the message to.",
            &["classifier", "folder", "tagging"],
            Undo::of_panel(
                weak,
                move |this| this.policy.mode(QuestionKey::Folder) != default_mode(QuestionKey::Folder),
                move |this, cx| this.set_mode(QuestionKey::Folder, default_mode(QuestionKey::Folder), cx),
            ),
            segmented(
                "ai-mode-group-Folder".to_owned(),
                vec![
                    ("ai-mode-Folder-off".to_owned(), "Off".to_owned()),
                    ("ai-mode-Folder-review".to_owned(), "Review".to_owned()),
                ],
                weak,
                move |this| usize::from(!matches!(this.policy.mode(QuestionKey::Folder), Mode::Off)),
                move |this, index, cx| {
                    let mode = if index == 0 { Mode::Off } else { Mode::Review };
                    this.set_mode(QuestionKey::Folder, mode, cx);
                },
            ),
        ));
        // The classifier switches. Both are off by default.
        tagging = tagging
            .item(row(
                "Classify new mail automatically",
                "Ask the classifier about incoming mail as it arrives.",
                &["classifier", "automatic", "auto classify", "tagging"],
                switch_undo(weak, |this| this.auto_classify, false, SettingsPanel::set_auto_classify),
                switch(weak, |this| this.auto_classify, SettingsPanel::set_auto_classify),
            ))
            .item(row(
                "Count cold outreach as spam",
                "Treat recruiters' and sales cold outreach as spam.",
                &["classifier", "spam", "cold outreach", "tagging"],
                switch_undo(weak, |this| this.spam_cold_outreach, false, SettingsPanel::set_spam_cold_outreach),
                switch(weak, |this| this.spam_cold_outreach, SettingsPanel::set_spam_cold_outreach),
            ));
        SettingPage::new("AI").resettable(true).groups([self.jev_group(weak), summaries, tagging])
    }
}

/// The inline message under the Connection row: a validation error, or the test result.
fn jev_message(error: Option<String>, test: Option<JevTest>, theme: &gpui_kit::component::Theme) -> Option<AnyElement> {
    if let Some(error) = error {
        return Some(
            div()
                .id("jev-error")
                .text_xs()
                .text_color(theme.danger)
                .child(Label::new(error))
                .into_any_element(),
        );
    }
    match test? {
        JevTest::Running => {
            Some(div().id("jev-test-status").text_xs().child(Label::new("Testing…")).into_any_element())
        }
        JevTest::Connected(model) => Some(
            div()
                .id("jev-test-status")
                .text_xs()
                .text_color(theme.primary)
                .child(Label::new(format!("Connected (model {model})")))
                .into_any_element(),
        ),
        JevTest::Failed(error) => Some(
            div()
                .id("jev-test-error")
                .text_xs()
                .text_color(theme.danger)
                .child(Label::new(error))
                .into_any_element(),
        ),
    }
}
/// No row ↺: the Jev rows are not values with a default the page reset puts back.
fn no_reset(weak: &Weak) -> Undo {
    Undo::of_panel(weak, |_| false, |_, _| {})
}

/// The confidence `question` ships with; what a reset puts back.
fn default_confidence(question: QuestionKey) -> Confidence {
    match default_mode(question) {
        Mode::Auto(confidence) => confidence,
        _ => Confidence::Medium,
    }
}

/// The stored key of `mode`, falling back to `default` while the row shows nothing.
fn confidence_of(mode: Mode, default: Confidence) -> SharedString {
    match mode {
        Mode::Auto(confidence) => confidence_key(confidence).into(),
        _ => confidence_key(default).into(),
    }
}

/// The Auto / Review / Off buttons of one question, each with an id no other row uses.
fn mode_entries(question: QuestionKey) -> Vec<(String, String)> {
    CLASSIFIER_MODES
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            (format!("ai-mode-{}-{index}", question.label()), (*label).to_owned())
        })
        .collect()
}
