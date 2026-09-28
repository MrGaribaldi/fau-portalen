//! Turning a selection of addresses into a `mailto:` link or a text to copy (groups design
//! §4.3). Pure: the caller has already authorized the selection, decrypted the contact
//! addresses and written the audit entry (`fau_persistence::membership::export_addresses`).
//!
//! - A person selected through two groups appears once, and so does an address two people
//!   share: [`Recipients::new`] removes repeats, keeping first-seen order.
//! - More than [`BCC_ABOVE`] recipients default to Bcc, so a large mailing does not hand
//!   every recipient everyone else's address.
//! - Mail clients (notably on Windows) truncate `mailto:` URLs at about 2,000 characters,
//!   so a link longer than [`MAILTO_MAX_CHARS`] is refused and the screen points to copying.
//! - Every address is written as an RFC 5322 `addr-spec`, with the local part quoted when it
//!   is not a dot-atom, so a `,` or `;` inside an address cannot split it into two.

use crate::email::Email;

/// More recipients than this default to Bcc (§4.3).
pub const BCC_ABOVE: usize = 10;

/// The longest `mailto:` URL the screen will open (§4.3). One more character switches the
/// screen to copying.
pub const MAILTO_MAX_CHARS: usize = 1800;

/// Which header of the new message the addresses go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Bcc,
}

impl RecipientField {
    /// Bcc above [`BCC_ABOVE`] recipients, To otherwise. The screen's toggle starts here.
    pub fn default_for(count: usize) -> Self {
        if count > BCC_ABOVE {
            RecipientField::Bcc
        } else {
            RecipientField::To
        }
    }
}

/// What "Kopier adresser" puts between addresses. `, ` is the RFC 6068 and RFC 5322 list
/// separator; `; ` is what Outlook desktop expects if the implementation-time check on the
/// screen card finds it needs one (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopySeparator {
    Comma,
    Semicolon,
}

impl CopySeparator {
    pub fn as_str(self) -> &'static str {
        match self {
            CopySeparator::Comma => ", ",
            CopySeparator::Semicolon => "; ",
        }
    }
}

/// A `mailto:` link, or the reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mailto {
    Link(String),
    /// The URL would be `length` characters, more than [`MAILTO_MAX_CHARS`]. "Skriv e-post"
    /// is disabled, and a short explanation points to "Kopier adresser".
    TooLong {
        length: usize,
    },
}

/// The addresses of one selection, each once, in first-seen order.
#[derive(Clone, PartialEq, Eq)]
pub struct Recipients(Vec<Email>);

impl std::fmt::Debug for Recipients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Recipients({} addresses)", self.0.len())
    }
}

impl Recipients {
    /// Removes repeats. `Email` is already trimmed and lower-cased, so equal addresses are
    /// equal strings.
    pub fn new(addresses: impl IntoIterator<Item = Email>) -> Self {
        let mut seen = std::collections::HashSet::new();
        Self(
            addresses
                .into_iter()
                .filter(|a| seen.insert(a.as_str().to_owned()))
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn default_field(&self) -> RecipientField {
        RecipientField::default_for(self.len())
    }

    /// `mailto:a@x.no,b@x.no` or `mailto:?bcc=a@x.no,b@x.no` (RFC 6068 §2: addresses are
    /// joined with a bare `,`; a space would have to be encoded and buys nothing).
    pub fn mailto(&self, field: RecipientField) -> Mailto {
        let joined = self
            .0
            .iter()
            .map(|a| percent_encode(&address_spec(a)))
            .collect::<Vec<_>>()
            .join(",");
        let url = match field {
            RecipientField::To => format!("mailto:{joined}"),
            RecipientField::Bcc => format!("mailto:?bcc={joined}"),
        };
        if url.len() > MAILTO_MAX_CHARS {
            Mailto::TooLong { length: url.len() }
        } else {
            Mailto::Link(url)
        }
    }

    /// The text "Kopier adresser" places on the clipboard, or in the fallback text box.
    pub fn copy_text(&self, separator: CopySeparator) -> String {
        self.0
            .iter()
            .map(address_spec)
            .collect::<Vec<_>>()
            .join(separator.as_str())
    }
}

/// RFC 5322 `atext`, plus any non-ASCII character (RFC 6531's `UTF8-non-ascii`).
fn is_atext(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c) || !c.is_ascii()
}

fn is_dot_atom(s: &str) -> bool {
    !s.is_empty()
        && s.split('.')
            .all(|part| !part.is_empty() && part.chars().all(is_atext))
}

/// The address as an RFC 5322 `addr-spec`: unchanged when its local part is a dot-atom,
/// otherwise with the local part quoted and `"` and `\` escaped.
pub fn address_spec(address: &Email) -> String {
    let (local, domain) = address
        .as_str()
        .rsplit_once('@')
        .expect("Email::parse guarantees an @");
    if is_dot_atom(local) {
        return address.as_str().to_owned();
    }
    let mut out = String::with_capacity(local.len() + domain.len() + 3);
    out.push('"');
    for c in local.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str("\"@");
    out.push_str(domain);
    out
}

/// Percent-encodes every byte except RFC 3986's unreserved characters and `@` (RFC 6068 §2:
/// encoding is always allowed, and `%`, `,`, `?`, `&`, `#`, `/` and non-ASCII must be).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> Email {
        Email::parse(s).unwrap()
    }

    fn many(n: usize) -> Recipients {
        Recipients::new((0..n).map(|i| e(&format!("forelder{i}@example.no"))))
    }

    #[test]
    fn repeats_are_removed_in_first_seen_order() {
        let r = Recipients::new([
            e("kari@example.no"),
            e("ola@example.no"),
            e("KARI@example.no"),
            e("kari@example.no"),
        ]);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.copy_text(CopySeparator::Comma),
            "kari@example.no, ola@example.no"
        );
    }

    #[test]
    fn bcc_is_the_default_above_ten() {
        assert_eq!(RecipientField::default_for(0), RecipientField::To);
        assert_eq!(RecipientField::default_for(10), RecipientField::To);
        assert_eq!(RecipientField::default_for(11), RecipientField::Bcc);
        assert_eq!(many(10).default_field(), RecipientField::To);
        assert_eq!(many(11).default_field(), RecipientField::Bcc);
        // Counted after repeats are removed: eleven selections of ten addresses stay To.
        let repeated =
            Recipients::new((0..11).map(|i| e(&format!("forelder{}@example.no", i % 10))));
        assert_eq!(repeated.default_field(), RecipientField::To);
    }

    #[test]
    fn to_and_bcc_links_join_with_a_bare_comma() {
        let r = Recipients::new([e("kari@example.no"), e("ola@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link("mailto:kari@example.no,ola@example.no".into())
        );
        assert_eq!(
            r.mailto(RecipientField::Bcc),
            Mailto::Link("mailto:?bcc=kari@example.no,ola@example.no".into())
        );
    }

    /// An address of exactly `n` characters, unique per `i`.
    fn address_of_len(i: usize, n: usize) -> String {
        let (tag, domain) = (format!("{i:04}"), "@example.no");
        format!("{tag}{}{domain}", "a".repeat(n - tag.len() - domain.len()))
    }

    /// Addresses whose `field` link is exactly `target` characters: 50-character addresses,
    /// then one that takes up the remainder.
    fn exactly(field: RecipientField, target: usize) -> Recipients {
        let mut len = match field {
            RecipientField::To => "mailto:".len(),
            RecipientField::Bcc => "mailto:?bcc=".len(),
        };
        let mut addresses = Vec::new();
        loop {
            let comma = usize::from(!addresses.is_empty());
            let left = target - len - comma;
            if left <= 100 {
                addresses.push(address_of_len(addresses.len(), left));
                break;
            }
            addresses.push(address_of_len(addresses.len(), 50));
            len += comma + 50;
        }
        Recipients::new(addresses.iter().map(|a| e(a)))
    }

    #[test]
    fn the_length_guard_switches_to_copying_above_1800_characters() {
        for field in [RecipientField::To, RecipientField::Bcc] {
            match exactly(field, MAILTO_MAX_CHARS).mailto(field) {
                Mailto::Link(url) => assert_eq!(url.len(), MAILTO_MAX_CHARS, "{field:?}"),
                other => panic!("{field:?}: {other:?}"),
            }
            assert_eq!(
                exactly(field, MAILTO_MAX_CHARS + 1).mailto(field),
                Mailto::TooLong {
                    length: MAILTO_MAX_CHARS + 1
                },
                "{field:?}"
            );
        }
    }

    #[test]
    fn the_guard_counts_encoded_characters() {
        // 'ø' is two UTF-8 bytes and six characters once percent-encoded.
        let r = Recipients::new([e("øystein@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link("mailto:%C3%B8ystein@example.no".into())
        );
    }

    #[test]
    fn addresses_that_are_not_dot_atoms_are_quoted_and_encoded() {
        let r = Recipients::new([
            e("kari+fau@example.no"),
            e("a,b@example.no"),
            e("per;paal@example.no"),
            e("q\"uote@example.no"),
            e(".dot@example.no"),
        ]);
        assert_eq!(
            r.copy_text(CopySeparator::Comma),
            "kari+fau@example.no, \"a,b\"@example.no, \"per;paal\"@example.no, \
             \"q\\\"uote\"@example.no, \".dot\"@example.no"
        );
        assert_eq!(
            r.copy_text(CopySeparator::Semicolon),
            "kari+fau@example.no; \"a,b\"@example.no; \"per;paal\"@example.no; \
             \"q\\\"uote\"@example.no; \".dot\"@example.no"
        );
        assert_eq!(
            r.mailto(RecipientField::To),
            Mailto::Link(
                "mailto:kari%2Bfau@example.no,%22a%2Cb%22@example.no,%22per%3Bpaal%22@example.no,\
                 %22q%5C%22uote%22@example.no,%22.dot%22@example.no"
                    .into()
            )
        );
    }

    #[test]
    fn url_delimiters_inside_an_address_are_encoded() {
        let r = Recipients::new([e("a?cc=x&b#c/d%e@example.no")]);
        assert_eq!(
            r.mailto(RecipientField::Bcc),
            Mailto::Link("mailto:?bcc=a%3Fcc%3Dx%26b%23c%2Fd%25e@example.no".into())
        );
    }

    #[test]
    fn debug_prints_the_count_only() {
        let r = Recipients::new([e("kari@example.no")]);
        assert_eq!(format!("{r:?}"), "Recipients(1 addresses)");
    }
}
