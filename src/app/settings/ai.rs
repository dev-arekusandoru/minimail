use super::*;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Toggle, ToggleGroup};

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
    pub(super) fn classifier_page(&self, weak: &Weak) -> SettingPage {
        let summaries = SettingGroup::new().item(
            SettingItem::new(
                "Thread summaries",
                switch(weak, |this| this.summaries, SettingsPanel::set_summaries).default_value(false),
            )
            .description("Write a short summary above each conversation.")
            .keywords(["summaries", "opt in to generated summaries"]),
        );
        let mut tagging = SettingGroup::new().title("Tagging");
        for question in TAGGED {
            tagging = tagging.item(
                SettingItem::new(question.label(), Self::mode_field(weak, question))
                    .description(tag_line(question))
                    .keywords([question.label(), "classifier", "auto review off", "tagging"]),
            );
            if matches!(self.policy.mode(question), Mode::Auto(_)) {
                tagging = tagging.item(
                    SettingItem::new(
                        format!("{} confidence", question.label()),
                        Self::confidence_field(weak, question),
                    )
                    .description("Apply without review once the classifier is this sure.")
                    .keywords([question.label(), "classifier confidence", "high medium low"]),
                );
            }
        }
        SettingPage::new("AI").resettable(false).groups([summaries, tagging])
    }

    /// Auto / Review / Off as one segmented control, with the reset marker shown
    /// while the row differs from the shipped default.
    fn mode_field(weak: &Weak, question: QuestionKey) -> SettingField<SharedString> {
        let (get, click) = (weak.clone(), weak.clone());
        let (dirty, reset) = (weak.clone(), weak.clone());
        SettingField::render(move |_, _, cx| {
            let selected = get
                .read_with(cx, |this, _| this.policy.mode(question))
                .map_or(0, mode_index);
            ToggleGroup::new(format!("ai-mode-{}", question.label()))
                .segmented()
                .small()
                .children(CLASSIFIER_MODES.iter().enumerate().map(|(index, (_, label))| {
                    Toggle::new(format!("ai-mode-{}-{index}", question.label()))
                        .label(*label)
                        .checked(index == selected)
                }))
                .on_click({
                    let click = click.clone();
                    move |next: &Vec<bool>, _, cx| {
                        // The group reports every segment's new state, so the pressed one is
                        // the segment whose state differs from the row's current mode.
                        let Ok(current) = click
                            .read_with(cx, |this, _| this.policy.mode(question))
                            .map(mode_index)
                        else {
                            return;
                        };
                        let Some(index) = (0..next.len()).find(|&i| next[i] != (i == current)) else {
                            return;
                        };
                        click
                            .update(cx, |this, cx| {
                                this.set_classifier_mode(question, CLASSIFIER_MODES[index].0, cx)
                            })
                            .ok();
                    }
                })
        })
        .on_reset(
            move |cx| {
                dirty
                    .read_with(cx, |this, _| this.policy.mode(question))
                    .map_or(true, |mode| mode != default_mode(question))
            },
            move |_, cx| {
                reset
                    .update(cx, |this, cx| this.set_mode(question, default_mode(question), cx))
                    .ok();
            },
        )
    }

    /// High / Medium / Low: the confidence Auto applies at without review.
    fn confidence_field(weak: &Weak, question: QuestionKey) -> SettingField<SharedString> {
        let (get, set) = (weak.clone(), weak.clone());
        let default = match default_mode(question) {
            Mode::Auto(confidence) => confidence,
            _ => Confidence::Medium,
        };
        SettingField::dropdown(
            options(&CONFIDENCES),
            move |cx| {
                get.read_with(cx, |this, _| match this.policy.mode(question) {
                    Mode::Auto(confidence) => confidence,
                    _ => Confidence::Medium,
                })
                .map_or(confidence_key(default).into(), |c| confidence_key(c).into())
            },
            move |value: SharedString, cx| {
                let confidence = confidence_from_key(&value);
                set.update(cx, |this, cx| this.set_confidence(question, confidence, cx)).ok();
            },
        )
        .default_value(confidence_key(default))
    }
}
