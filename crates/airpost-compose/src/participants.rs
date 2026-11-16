// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Who a reply goes to: `Message.participantsForReply()` and `participantsForReplyAll()`
//! (`message.ts`), `DraftFactory.updateDraftForReply` for a reply draft switched between the
//! two, and `Utils.subjectWithPrefix`.

use crate::contacts::{equivalent_email, Contact};
use crate::identity::Identities;

/// The recipients of a message being replied to.
#[derive(Clone, Debug, Default)]
pub struct Replied {
    pub from: Vec<Contact>,
    pub to: Vec<Contact>,
    pub cc: Vec<Contact>,
    pub reply_to: Vec<Contact>,
}

impl Replied {
    pub fn from_message_json(m: &serde_json::Value) -> Replied {
        let list = |k: &str| m.get(k).map(Contact::list_from_json).unwrap_or_default();
        Replied {
            from: list("from"),
            to: list("to"),
            cc: list("cc"),
            reply_to: list("replyTo"),
        }
    }

    fn is_from_me(&self, me: &Identities) -> bool {
        self.from.first().is_some_and(|f| me.is_me(&f.email))
    }

    fn reply_to_other(&self, me: &Identities) -> bool {
        self.reply_to.first().is_some_and(|r| !me.is_me(&r.email))
    }
}

fn uniq(list: Vec<Contact>) -> Vec<Contact> {
    let mut seen: Vec<String> = Vec::new();
    list.into_iter()
        .filter(|c| {
            let key = equivalent_email(&c.email);
            if seen.contains(&key) {
                return false;
            }
            seen.push(key);
            true
        })
        .collect()
}

/// `participantsForReply()`: `(to, cc)`.
pub fn for_reply(m: &Replied, me: &Identities) -> (Vec<Contact>, Vec<Contact>) {
    let to = if m.reply_to_other(me) {
        m.reply_to.clone()
    } else if m.is_from_me(me) {
        m.to.clone()
    } else {
        m.from.clone()
    };
    (uniq(to), Vec::new())
}

/// `participantsForReplyAll()`: `(to, cc)`.
pub fn for_reply_all(m: &Replied, me: &Identities) -> (Vec<Contact>, Vec<Contact>) {
    let excluded: Vec<String> = m.from.iter().map(|c| equivalent_email(&c.email)).collect();
    let exclude_me_and_froms = |list: Vec<Contact>| -> Vec<Contact> {
        list.into_iter()
            .filter(|p| !(me.is_me(&p.email) || excluded.contains(&equivalent_email(&p.email))))
            .collect()
    };
    let both = || [m.to.clone(), m.cc.clone()].concat();
    let (to, cc) = if m.reply_to_other(me) {
        (m.reply_to.clone(), exclude_me_and_froms(both()))
    } else if m.is_from_me(me) {
        (m.to.clone(), exclude_me_and_froms(m.cc.clone()))
    } else {
        (m.from.clone(), exclude_me_and_froms(both()))
    };
    (uniq(to), uniq(cc))
}

/// `DraftFactory.updateDraftForReply`: an existing reply draft's `(to, cc)` for the new kind.
/// Switching to reply drops the reply-all-only participants; switching to reply-all adds the
/// missing ones and never drops anyone.
pub fn update_for_reply(
    draft_to: &[Contact],
    draft_cc: &[Contact],
    m: &Replied,
    me: &Identities,
    reply_all: bool,
) -> (Vec<Contact>, Vec<Contact>) {
    let (reply_to, reply_cc) = for_reply(m, me);
    let (all_to, all_cc) = for_reply_all(m, me);
    let mut to = draft_to.to_vec();
    let mut cc = draft_cc.to_vec();
    if !reply_all {
        let keep = |list: &mut Vec<Contact>, reply: &[Contact], all: &[Contact]| {
            list.retain(|c| {
                let in_reply = reply.iter().any(|x| x.email == c.email);
                let in_all = all.iter().any(|x| x.email == c.email);
                !in_all || in_reply
            });
        };
        keep(&mut to, &reply_to, &all_to);
        keep(&mut cc, &reply_cc, &all_cc);
    }
    let (target_to, target_cc) = if reply_all {
        (all_to, all_cc)
    } else {
        (reply_to, reply_cc)
    };
    for (list, target) in [(&mut to, target_to), (&mut cc, target_cc)] {
        for c in target {
            if !list.iter().any(|x| x.email == c.email) {
                list.push(c);
            }
        }
    }
    (to, cc)
}

/// `Utils.subjectWithPrefix`: a leading `Fwd:` or `Re:` (any case) becomes `prefix`, else
/// `prefix` is put in front.
pub fn subject_with_prefix(subject: &str, prefix: &str) -> String {
    let lower = subject.to_lowercase();
    for p in ["fwd:", "re:"] {
        if lower.starts_with(p) {
            return format!("{prefix}{}", &subject[p.len()..]);
        }
    }
    format!("{prefix} {subject}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use serde_json::json;

    fn me() -> Identities {
        Identities(vec![Identity::from_account_json(&json!({
            "id": "a1", "name": "Ann", "emailAddress": "ann@example.com",
            "aliases": ["Work <ann@work.example>"],
        }))])
    }

    #[test]
    fn reply_and_reply_all() {
        let m = Replied::from_message_json(&json!({
            "from": [{"name": "Bob", "email": "bob@y.example"}],
            "to": [{"email": "ann@example.com"}, {"email": "carol@y.example"}],
            "cc": [{"email": "Bob+list@y.example"}, {"email": "dan@y.example"}],
        }));
        let (to, cc) = for_reply(&m, &me());
        assert_eq!(to, vec![Contact::new("Bob", "bob@y.example")]);
        assert!(cc.is_empty());
        let (to, cc) = for_reply_all(&m, &me());
        assert_eq!(to.len(), 1);
        let emails: Vec<&str> = cc.iter().map(|c| c.email.as_str()).collect();
        assert_eq!(emails, ["carol@y.example", "dan@y.example"]);
        let (to, cc) = update_for_reply(&to, &cc, &m, &me(), false);
        assert_eq!(to.len(), 1);
        assert!(cc.is_empty());
    }

    #[test]
    fn subjects() {
        assert_eq!(subject_with_prefix("Hello", "Re:"), "Re: Hello");
        assert_eq!(subject_with_prefix("RE: Hello", "Re:"), "Re: Hello");
        assert_eq!(subject_with_prefix("FWD: Hello", "Re:"), "Re: Hello");
        assert_eq!(subject_with_prefix("Re: Hello", "Fwd:"), "Fwd: Hello");
    }
}
