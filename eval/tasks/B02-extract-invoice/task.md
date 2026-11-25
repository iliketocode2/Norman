# B02 · Extract an invoice

Define `(invoice-total model document)`.

Ask `model` once to extract an `Invoice` (defined for you) from `document`, a
`Text` holding an invoice. Then answer with the sum of the `cents` of its
items, as a `Num`. Check the extracted invoice before adding it up:

- an invoice with no items fails with `(fail 'no-items)`;
- an invoice with any item whose `cents` is negative fails with
  `(fail 'negative-amount)`.

Don't ask again: if the reply doesn't fit the type, the `Invalid` failure
propagates, as does any other failure.

**Sites:** the `ask` uses the site `'invoice`.

**Example:** if the model replies
`{"vendor":"Acme","items":[{"description":"Bolts","cents":1250},{"description":"Nuts","cents":300}]}`,
the answer is `1550`.
