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
        SettingPage::new("AI").resettable(true).groups([summaries, tagging])
    }
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
