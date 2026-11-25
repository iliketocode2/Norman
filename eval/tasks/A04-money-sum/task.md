# A04 · Invoice total

Define `(invoice-total items)`.

`items` is a list of `Item` records (defined for you). The **subtotal** is the
sum of `price × qty` over all items. If the subtotal is **at least**
`$100.00`, a 10% discount applies: the total is the subtotal minus a tenth of
the subtotal, where the tenth is rounded down to a whole micro-dollar.
Otherwise the total is the subtotal. Answer with the total, as `Money`.

An empty list totals `$0`.

**Sites:** none. This task makes no `ask`, `call` or `consult`.

**Example:**

```
(invoice-total (list (Item [name "pen"] [price $1.50] [qty 4])
                     (Item [name "pad"] [price $3.00] [qty 1])))
;; is $9.00
```
