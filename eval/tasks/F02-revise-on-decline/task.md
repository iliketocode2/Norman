# F02 · Revise until approved

Define `(draft-approved model reviewer brief)`.

Ask `model` for a draft following `brief` (a `Text`), of type `(Text 300)`.
Consult `reviewer` about it, for an answer of the same type, **proposing**
the draft. Answer with what the person gives back: the draft if they approve
it, or their edit.

If the person declines with a reason, ask `model` for a revised draft, with
the previous draft (as an `Assistant` message) and the reason (as a `User`
message) added to the end of the context, and consult again. Allow at most
**two** revisions, so at most three drafts and three consults. If the third
draft is declined too, its `Declined` failure propagates.

Every other failure propagates.

**Sites:** every draft uses the site `'draft`, and every consult uses
`'review`.

**Example:** if the first draft is declined with `"Too long."`, and the
person approves the second draft, `"Short and sweet."`, the answer is
`"Short and sweet."`.
