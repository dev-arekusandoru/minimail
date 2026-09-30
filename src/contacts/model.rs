//! Plain data types: a stored contact, the fields used to create or replace
//! one, and its child collections. No ORM, no lazy loading.

use crate::clock::Timestamp;

/// Where a contact came from. Stored as text so the DB stays readable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContactSource {
    #[default]
    Manual,
    /// Created by allowing a sender in the Screener.
    Screener,
    /// Shipped in the seed fixture.
    Seed,
    /// Bulk import from another address book.
    Import,
}

impl ContactSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ContactSource::Manual => "manual",
            ContactSource::Screener => "screener",
            ContactSource::Seed => "seed",
            ContactSource::Import => "import",
        }
    }

    pub fn parse(s: &str) -> ContactSource {
        match s {
            "screener" => ContactSource::Screener,
            "seed" => ContactSource::Seed,
            "import" => ContactSource::Import,
            _ => ContactSource::Manual,
        }
    }
}

/// What an email, phone, postal address or URL is for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Label {
    Home,
    Work,
    Other,
    /// Any free-text label the provider gave us.
    Custom(String),
    #[default]
    Unlabeled,
}

impl Label {
    pub fn as_str(&self) -> &str {
        match self {
            Label::Home => "home",
            Label::Work => "work",
            Label::Other => "other",
            Label::Custom(s) => s,
            Label::Unlabeled => "",
        }
    }

    pub fn parse(s: &str) -> Label {
        match s {
            "home" => Label::Home,
            "work" => Label::Work,
            "other" => Label::Other,
            "" => Label::Unlabeled,
            other => Label::Custom(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailAddress {
    pub address: String,
    pub label: Label,
    pub primary: bool,
}

impl EmailAddress {
    pub fn new(address: impl Into<String>) -> Self {
        Self { address: address.into(), label: Label::Unlabeled, primary: false }
    }

    pub fn primary(address: impl Into<String>) -> Self {
        Self { address: address.into(), label: Label::Unlabeled, primary: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneNumber {
    pub number: String,
    pub label: Label,
    pub primary: bool,
}

impl PhoneNumber {
    pub fn new(number: impl Into<String>) -> Self {
        Self { number: number.into(), label: Label::Unlabeled, primary: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostalAddress {
    pub street: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
    pub label: Label,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactUrl {
    pub url: String,
    pub label: Label,
}

/// A contact as stored, with its child collections loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub id: i64,
    pub prefix: Option<String>,
    pub given_name: Option<String>,
    pub middle_name: Option<String>,
    pub family_name: Option<String>,
    pub suffix: Option<String>,
    pub nickname: Option<String>,
    pub display_name: String,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    /// `YYYY-MM-DD` when known.
    pub birthday: Option<String>,
    pub notes: Option<String>,
    /// Path or URL of a photo.
    pub photo: Option<String>,
    pub favorite: bool,
    pub source: ContactSource,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_contacted_at: Option<Timestamp>,
    pub emails: Vec<EmailAddress>,
    pub phones: Vec<PhoneNumber>,
    pub addresses: Vec<PostalAddress>,
    pub urls: Vec<ContactUrl>,
    /// Group names, alphabetically.
    pub groups: Vec<String>,
}

impl Contact {
    /// The address the UI would show for this contact.
    pub fn primary_email(&self) -> Option<&str> {
        self.emails
            .iter()
            .find(|e| e.primary)
            .or_else(|| self.emails.first())
            .map(|e| e.address.as_str())
    }
}

/// Fields for [`crate::contacts::ContactStore::create`] and `update`. Absent
/// optional fields are stored as NULL; a non-empty `emails`/`phones`/
/// `addresses`/`urls`/`groups` list replaces the stored children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewContact {
    pub prefix: Option<String>,
    pub given_name: Option<String>,
    pub middle_name: Option<String>,
    pub family_name: Option<String>,
    pub suffix: Option<String>,
    pub nickname: Option<String>,
    pub display_name: String,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub birthday: Option<String>,
    pub notes: Option<String>,
    pub photo: Option<String>,
    pub favorite: bool,
    pub source: ContactSource,
    pub emails: Vec<EmailAddress>,
    pub phones: Vec<PhoneNumber>,
    pub addresses: Vec<PostalAddress>,
    pub urls: Vec<ContactUrl>,
    pub groups: Vec<String>,
}

impl NewContact {
    /// A contact with just a display name; everything else empty.
    pub fn new(display_name: impl Into<String>) -> Self {
        Self { display_name: display_name.into(), ..Self::default() }
    }

    /// The minimal contact the Screener creates for an unknown sender.
    pub fn from_email(email: impl Into<String>, display_name: impl Into<String>) -> Self {
        let mut c = Self::new(display_name);
        c.emails.push(EmailAddress::primary(email));
        c
    }

    pub fn with(mut self, email: EmailAddress) -> Self {
        self.emails.push(email);
        self
    }

    pub fn organization(mut self, org: impl Into<String>) -> Self {
        self.organization = Some(org.into());
        self
    }

    pub fn job_title(mut self, title: impl Into<String>) -> Self {
        self.job_title = Some(title.into());
        self
    }

    pub fn nickname(mut self, name: impl Into<String>) -> Self {
        self.nickname = Some(name.into());
        self
    }

    pub fn phone(mut self, number: PhoneNumber) -> Self {
        self.phones.push(number);
        self
    }

    pub fn group(mut self, name: impl Into<String>) -> Self {
        self.groups.push(name.into());
        self
    }

    pub fn favorite(mut self, yes: bool) -> Self {
        self.favorite = yes;
        self
    }

    pub fn source(mut self, source: ContactSource) -> Self {
        self.source = source;
        self
    }
}

/// A label that groups contacts, e.g. "Work".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
}

/// A `contact_fields` key/value pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub value: String,
}

/// Filter for [`crate::contacts::ContactStore::search`]. All fields are ANDed;
/// `limit`/`offset` paginate in display-name order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContactQuery {
    /// Case-insensitive prefix of display name, email or organization.
    pub text: Option<String>,
    pub favorites_only: bool,
    /// Contacts in this group.
    pub group: Option<String>,
    pub limit: Option<usize>,
    pub offset: usize,
}

impl ContactQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn favorites(mut self, yes: bool) -> Self {
        self.favorites_only = yes;
        self
    }

    pub fn group(mut self, name: impl Into<String>) -> Self {
        self.group = Some(name.into());
        self
    }

    pub fn page(mut self, limit: usize, offset: usize) -> Self {
        self.limit = Some(limit);
        self.offset = offset;
        self
    }
}
