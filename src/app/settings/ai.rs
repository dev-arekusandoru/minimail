use super::*;

impl SettingsPanel {
    pub(super) fn classifier_page(&self, weak: &Weak) -> SettingPage {
        let groups = QuestionKey::ALL.into_iter().map(|question| {
            let mode_field = {
                let (get, set) = (weak.clone(), weak.clone());
                SettingField::dropdown(
                    options(&CLASSIFIER_MODES),
                    move |cx| {
                        let index = get
                            .read_with(cx, |this, _| match this.policy.mode(question) {
                                Mode::Auto(_) => 0,
                                Mode::Review => 1,
                                Mode::Off => 2,
                            })
                            .unwrap_or_default();
                        pick(&CLASSIFIER_MODES, index)
                    },
                    move |value: SharedString, cx| {
                        set.update(cx, |this, cx| this.set_classifier_mode(question, &value, cx)).ok();
                    },
                )
                .default_value(SharedString::from(match question {
                    QuestionKey::Spam => "auto",
                    QuestionKey::NeedsReply => "auto",
                    QuestionKey::ExpectsReply => "review",
                    QuestionKey::Urgency => "auto",
                    QuestionKey::Kind => "auto",
                }))
            };
            let confidence_field = {
                let (get, set) = (weak.clone(), weak.clone());
                SettingField::dropdown(
                    options(&CONFIDENCES),
                    move |cx| {
                        let index = get
                            .read_with(cx, |this, _| match this.policy.mode(question) {
                                Mode::Auto(Confidence::High) => 0,
                                Mode::Auto(Confidence::Low) => 2,
                                _ => 1,
                            })
                            .unwrap_or_default();
                        pick(&CONFIDENCES, index)
                    },
                    move |value: SharedString, cx| {
                        let confidence = match &*value {
                            "high" => Confidence::High,
                            "low" => Confidence::Low,
                            _ => Confidence::Medium,
                        };
                        set.update(cx, |this, cx| this.set_confidence(question, confidence, cx)).ok();
                    },
                )
                .default_value(SharedString::from(match question {
                    QuestionKey::Spam => "high",
                    QuestionKey::Kind => "low",
                    _ => "medium",
                }))
            };
            let auto = matches!(self.policy.mode(question), Mode::Auto(_));
            SettingGroup::new().title(question.label()).items([
                SettingItem::new("Handling", mode_field)
                    .description("Choose automatic handling, manual review, or off.")
                    .keywords([question.label(), "auto apply or review", "classifier"]),
                SettingItem::new("Confidence", confidence_field)
                    .description("How sure the classifier must be for automatic handling.")
                    .keywords([question.label(), "classifier confidence"])
                    .disabled(!auto),
            ])
        });
        SettingPage::new("AI").resettable(false).groups(
            std::iter::once(
                SettingGroup::new().item(
                    SettingItem::new(
                        "Thread summaries",
                        switch(weak, |this| this.summaries, SettingsPanel::set_summaries).default_value(false),
                    )
                    .description("Opt in to generated summaries above conversations.")
                    .keywords(["opt in to generated summaries"]),
                ),
            )
            .chain(groups)
            .collect::<Vec<_>>(),
        )
    }
}
