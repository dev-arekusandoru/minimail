# Deferred email composer changes

Features deliberately outside the initial Gmail sending and composer work. Each
entry says what shipped now and what is still open, so the next cut can pick it
up without re-deriving the decision.

- [ ] **Signatures** — add signature editing and insertion into composed
      messages. Nothing is inserted today. Still to decide: where signatures are
      stored, whether they are account-specific, whether they sit above or below
      the quoted original, and how they interact with a Markdown body.
- [ ] **Live formatted editor** — offer a WYSIWYG surface as an optional
      composer setting. The composer is Markdown source today, with `cmd-shift-v`
      previewing the exact message that will be sent. Still to decide: the
      editor component, and how its document round-trips to Markdown.
- [ ] **Pasted rich-text fidelity** — pasted HTML is converted to Markdown, so
      fonts, colours, and per-run sizes are dropped. Marks that Markdown can
      express (bold, italic, links, lists, headings, quotes, code, tables,
      strikethrough) survive. Preserving the rest needs a body format richer
      than Markdown, or an HTML part carried alongside it.
- [ ] **Send later** — schedule a send for a chosen time. Only the 10-second
      undo hold exists; a hold is never requeued for later, and an interrupted
      hold comes back as an editable draft rather than a scheduled send.
- [ ] **Templates and snippets** — reusable bodies and reusable text inserted
      from the palette. Nothing is stored or inserted yet.
- [ ] **Hard flush on quit** — when the main window closes, dirty drafts are
      marked urgent and their writes are *started* on the background local lane
      (`flush_dirty_drafts`), but that lane's thread is not joined and the process
      does not wait for it, so the last write is best-effort: a quit that tears
      the process down immediately can still lose the final keystrokes. A hard
      guarantee needs the close path to join/flush the local lane before the
      window goes away (or a synchronous local write there); the seam is
      `MailApp::watch_window_close` / `flush_dirty_drafts`.
