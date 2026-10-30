//! What a composer's recipient fields know: everyone the app has seen (address book and
//! mailbox) for the autocomplete and the NEW badge, and the people a reply-all started with.

use super::*;
use crate::app::compose::recipients::Field;
use crate::contacts::ContactQuery;
use crate::outgoing::DraftRecord;
use crate::reading::parse_recipients;
use crate::recipient_suggest::Directory;

impl MailApp {
    /// Contacts plus every address in the mailbox. A message you received credits its
    /// sender, one you sent credits its recipients; the rest of its header only becomes
    /// suggestable.
    fn recipient_directory(&self) -> Directory {
        let mut directory = Directory::default();
        if let Some(Ok(contacts)) = self
            .preferences
            .as_ref()
            .map(|store| store.search(&ContactQuery::default()))
        {
            for contact in contacts {
                for email in &contact.emails {
                    directory.add_contact(&contact.display_name, &email.address);
                }
            }
        }
        for m in self.mailbox.messages() {
            let sender = parse_recipients(&format!("{} <{}>", m.from_name, m.from_email));
            let rest: Vec<_> = [&m.to, &m.cc, &m.bcc]
                .into_iter()
                .flat_map(|f| parse_recipients(f))
                .collect();
            if m.outgoing {
                directory.add_mail(m.thread_id, &rest);
                directory.add_seen(m.thread_id, &sender);
            } else {
                directory.add_mail(m.thread_id, &sender);
                directory.add_seen(m.thread_id, &rest);
            }
        }
        directory
    }

    /// Hand a freshly built composer its directory and, for a reply-all, the recipients the
    /// thread started with.
    pub(super) fn give_recipient_context(
        &self,
        record: &DraftRecord,
        composer: &Entity<crate::app::compose::Composer>,
        cx: &mut Context<Self>,
    ) {
        let baseline = (record.kind == DraftKind::ReplyAll)
            .then(|| {
                let message = self.mailbox.get(record.source_message?)?;
                let own = self
                    .mailbox
                    .account(&message.account)
                    .map(|a| a.email.clone())
                    .unwrap_or_default();
                let draft = crate::draft::draft(crate::draft::DraftKind::ReplyAll, message, &own);
                let to = crate::outgoing::split_addresses(&draft.to)
                    .into_iter()
                    .map(|c| (Field::To, c));
                let cc = crate::outgoing::split_addresses(&draft.cc)
                    .into_iter()
                    .map(|c| (Field::Cc, c));
                Some(to.chain(cc).collect::<Vec<_>>())
            })
            .flatten()
            .unwrap_or_default();
        let directory = Rc::new(self.recipient_directory());
        composer.update(cx, |c, cx| c.set_recipient_context(directory, baseline, cx));
    }
}
