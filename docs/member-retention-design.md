# Remembering a former member: design

Status: **agreed in chat with Erik, 29 September 2026; not built.** It follows #3502 (the member
directory) and amends Erik's decision D3: a display name is shown only while a membership is
active. The name is no longer destroyed the moment the membership ends. It is kept hidden for a
period the member chooses, so that a person who comes back is recognised.

## 1. What Erik asked for

"We should have a way to store a user with display name for a few months, but have the FAU only
show the role and year. So if someone is a member of a FAU for a year, then they exit, we update
all documents in the FAU to role and year. But then the user comes back as a guest to continue
work on something, we should remember them. The user should be able to configure how long we
store the data."

## 2. Decisions

| # | Question | Decision (Erik, 29 September) |
|---|---|---|
| M1 | What others see on a returner's old contributions | **Role and year, always.** Only the returning person is recognised: their name, their contact address, and their own open work. The link between them and their past contributions is never shown to others |
| M2 | Retention period | **3 months by default.** The member can choose none, 3, 6, 12 or 24 months |
| M3 | Scope | **Per FAU.** Each FAU remembers its own former member, encrypted under that FAU's record key. Deleting or crypto-shredding the FAU removes it |
| M4 | Timing | **A follow-up card after #3502 is merged**, with migration 0009 |

## 3. States of a membership's profile

| State | Name and contact address stored | Shown to other members | History labels |
|---|---|---|---|
| Active | yes | yes | name for events in this active period |
| **Retained** (ended, within the period) | yes, hidden | **no**: the directory, export and name lookup treat the person as ended | role and year |
| Expired (period over) | no: the sweep clears both | no | role and year |
| Erased (Article 17) | no, at once, whatever the period | no | "Tidligere medlem" |

- The period starts when the membership ends (D3's definition: revoked, or no running or upcoming
  role), and runs for the member's `accounts.retention_months`.
- `retention_months = 0` means today's #3502 behaviour: clear at once.
- **Coming back within the period** (a new invitation, a guest role, or a new role granted)
  reopens the same membership row (`ensure_membership` already does this). The retained name
  becomes the current name again, and acceptance can prefill it. #3502's `existing_current`
  becomes "current or retained". The #3502 fix to `grant_role` (clear on grant to an ended
  membership) changes to: restore if retained, clear if expired.
- **Coming back after the period** is today's behaviour: the person states a name again.

## 4. What changes in the code

- **Migration 0009**:
  - Replace 0008's two `*_only_while_current` checks with "only while current, or while
    retained": a revoked row may keep its fields while `profile_retained_until` is in the future.
  - Add `memberships.profile_retained_until` (date, null while active).
  - Change `accounts.retention_months` to `between 0 and 24`, and restrict it to the allowed
    values {0, 3, 6, 12, 24}.
- **Ending a membership** (`revoke_membership`, and the point at which roles run out): set
  `profile_retained_until = end date + retention_months` instead of clearing. When the period is
  0, clear as now.
- **The sweep** (`clear_ended_profiles`) clears only when `profile_retained_until` has passed, or
  when the membership ended with a period of 0.
- **Reads:**
  - The directory, export and `member_names` for others are unchanged: they already decide "ended"
    at read time and never show an ended membership's name. That rule stays binding.
  - **History labels by period.** With M1, an event from an earlier active period stays role and
    year even after the person returns. `member_names` must label by the membership period the
    event fell in, not by whether the membership is active today. This needs the periods to be
    derivable from role assignments (they are), and it is the largest change.
- **The member's own setting:** a `set_retention_months` for the account, own account only, with
  allowed values checked. Changing it recalculates `profile_retained_until` on that account's
  retained memberships. It never extends a period that has already expired.
- **Erasure** (#3426) clears everything at once, retained or not.
- **The account itself.** ADR-003 §6a already keeps a login email "while any active membership
  exists, lapsing 3 months after the last one ends". The same `retention_months` governs that
  lapse, so a remembered membership never outlives its account's email. Check that on #3426.

## 5. Privacy

This is a new retention statement. The privacy notice and the DPA (#3426) must say:
- a former member's name and contact address are kept hidden for their chosen period, 3 months
  by default;
- they are used only to recognise the member if they return;
- others see role and year;
- the member can shorten the period to none at any time.

## 6. Tests the plan must include

- Ending with each period value.
- The sweep before and after the period ends.
- A return within the period (name restored, others still see role and year on old events).
- A return after the period (no name).
- `retention_months = 0`.
- Erasure during retention.
- Cross-tenant isolation.
- A guest who returns sees only their own groups.
- Changing the setting shortens an active retention.
- History labels for events from an earlier active period after a return.
