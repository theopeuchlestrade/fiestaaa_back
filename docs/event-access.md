# Event access

The API enforces access to protected event data and operations. The frontend
uses these rules when showing modules and actions. Opening a module through a
direct link does not grant access.

## Membership and lifecycle

- The organizer (event owner) and guests who have accepted their invitations can
  access protected module lists such as items, polls, expenses and carpools.
  A pending invitation does not grant access.
- Authorized members can still read protected module data after the event ends.
  Protected writes that require an active event fail with `event_finished`.

## Module permissions

| Module | Rules |
| --- | --- |
| Items | Only the organizer can create needs. Accepted guests can create personal brings and contribute to existing needs within quantity limits. A contribution belongs to the existing need. The organizer or the item's creator can delete the item. |
| Polls | Only the organizer can create polls. Accepted members can vote in active polls, subject to the poll's single- or multiple-choice settings. |
| Expenses | Expense participants must be eligible members of the event. Only the organizer, payer or expense creator can delete an expense. |
| Tickets | Only the organizer can scan tickets. Ticket issuance checks the invitation and event lifecycle. Scanning a consumed ticket returns `already_scanned`. |

See the [membership and lifecycle checks](../src/routes/event_access.rs) and the
handlers for [items](../src/routes/events/items.rs), [polls](../src/routes/events/polls.rs),
[expenses](../src/routes/events/expenses.rs), [carpools](../src/routes/carpools.rs)
and [tickets](../src/routes/qr_codes.rs). The
[push delivery guide](testing/push-delivery.md#poll-notification-access) documents
who receives poll notifications.
