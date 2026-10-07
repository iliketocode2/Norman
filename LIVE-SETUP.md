# Running µNorman against a real model

To point `ask` at a real Anthropic API:


## 1. Get a credential

Two options. Either works with the existing client; pick one.

### Option A: an API key (simplest)

1. Go to the Claude Console at **<https://platform.claude.com>** and sign in.
   If you don't have an account, create one. API usage is billed separately
   from any Claude subscription, so you'll need billing set up with a payment
   method or prepaid credits.
2. Go to **<https://platform.claude.com/settings/keys>** (Settings → API keys).
3. Click **Create Key**, give it a name such as `munorman-dev`, and choose a
   workspace. A workspace with its own spend limit is a good idea for
   experiments.
4. **Copy the key immediately.** It's shown once and starts with `sk-ant-`.
   If you lose it, create another one and archive the old one.

Treat the key like a password. It is not scoped to this project, and anyone
holding it can spend money on your account.

### Option B: OAuth through the `ant` CLI

If you'd rather not handle a long-lived key:

```bash
ant auth login                               # opens a browser, stores a profile
ant auth status                              # confirms which profile is active
ant auth print-credentials --access-token    # prints a short-lived token
```

µNorman has no credential resolution of its own, so you pass the token through
the environment (next section). The token is short-lived, so this is good for
a session of work and bad for an unattended run.

---

## 2. Setting it in the environment

µNorman reads three variables ([`src/live.rs`](src/live.rs)):

| Variable | Meaning |
|---|---|
| `ANTHROPIC_API_KEY` | sent as `x-api-key`. Checked first. |
| `ANTHROPIC_AUTH_TOKEN` | sent as `Authorization: Bearer …`, with the OAuth beta header. Used only if the key is unset. |
| `ANTHROPIC_BASE_URL` | overrides the endpoint. This is how the stub server is reached in tests. |

**PowerShell**, for the current session:

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
```

**PowerShell**, persistently for your user (new shells only):

```powershell
[Environment]::SetEnvironmentVariable("ANTHROPIC_API_KEY", "sk-ant-...", "User")
```

**Git Bash / WSL**, for the current session:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Don't put the key in a file inside the repository. If you want it on disk, put
it somewhere outside the working tree and load it from your shell profile.
`.gitignore` is not a security control.

---

## 3. Running

```bash
cargo run -- --live --trace FILE.nrm
```

- `--live` sends every `ask` to the real model. Without it, asks are answered
  from scripts and nothing leaves the machine.
- `--trace` prints one line per ask or call: site, tokens, cost, timing and
  outcome. Use it every time in live mode. It is the only way to see what you
  actually spent.

`cargo test` never calls the real API. The live tests all go through a stub
server bound to `127.0.0.1`.

### Keeping the spend small

Live mode still enforces the budget, so an `under` or `budget` limit is a real
cap, not a suggestion:

```scheme
(under ([cost $0.05] [time 1min])
  (check-expect (ask-analyst) (Sell "…")))
```