# Serving the RunwayBar landing site at `runwaybar.applair.in`

This is the exact, ordered runbook for routing the GitHub Pages site of
`nitishagar/runwaybar` to a subdomain of `applair.in` (which lives on Cloudflare).
Order matters: GitHub's own docs say to configure the custom domain in the repo's
Pages settings **before** creating the DNS record, and to keep the verification
TXT record permanently.

## Preconditions

- The repo is **public** (GitHub Pages on the free plan requires public repositories;
  the `pages` workflow deploys `site/` on every push to `main`).
- The Pages site is already building: repo → Settings → Pages → Source is
  "GitHub Actions", and the `pages` workflow has run green at least once
  (verify the deployment URL `https://nitishagar.github.io/runwaybar/` loads).

## Steps (in order)

1. **Add the custom domain in the repo (before DNS).**
   Repo → Settings → Pages → Custom domain → enter `runwaybar.applair.in` → Save.
   GitHub will show a "DNS check in progress" state — that is expected until step 3.
   Because the site deploys via a custom Actions workflow, GitHub manages the domain
   binding; you do **not** need a `CNAME` file in `site/` (with custom workflows a
   CNAME file is ignored anyway).

2. **Create the verification TXT record (keep it permanently).**
   If Pages shows a verification value, add in Cloudflare (DNS → Records → Add):
   - Type `TXT`, Name `_github-pages-challenge-nitishagar.runwaybar`, Content `<value from Pages settings>`, TTL Auto.
   GitHub's docs require this record to stay in place permanently — removing it can
   orphan the domain binding.

3. **Create the CNAME record.**
   - Type `CNAME`, Name `runwaybar`, Target `nitishagar.github.io`, TTL Auto.
   - Proxy status: either works, with different trade-offs:
     - **DNS only (grey cloud)** — simplest: GitHub provisions its own certificate,
       HTTPS just works, and "Enforce HTTPS" becomes available in Pages settings.
     - **Proxied (orange cloud)** — Cloudflare fronts the site (hides origin, adds
       caching/WAF). Then Cloudflare's certificate serves visitors and GitHub cannot
       issue its own cert; "Enforce HTTPS" in Pages settings may stay unavailable —
       that is fine because Cloudflare terminates TLS.

4. **Check the zone's SSL/TLS mode (important — it affects the apex site too).**
   Cloudflare → SSL/TLS → Overview:
   - Mode **Full** or **Full (strict)**: nothing to do.
   - Mode **Flexible**: do NOT change it zone-wide (the apex `applair.in` site depends
     on it). Instead add a hostname-scoped rule: Rules → Configuration rules →
     Create rule matching hostname `runwaybar.applair.in` → set SSL to **Full (strict)**.
     Flexible at the edge against GitHub Pages causes redirect loops.
   - If the zone uses the newer "Automatic SSL/TLS", verify the effective mode for the
     hostname follows the same logic (Full, never Flexible, toward GitHub).

5. **Enable HTTPS enforcement (DNS-only setups).**
   Back in repo Settings → Pages: once GitHub's certificate is issued (can take up to
   24 h), tick **Enforce HTTPS**. With a proxied record, skip this — Cloudflare handles
   HTTPS at the edge (redirect HTTP→HTTPS there if desired).

6. **Verify nothing else in the zone changed.**
   The apex `applair.in` A/AAAA records, the `route1-3.mx.cloudflare.net` MX entries
   and the SPF TXT (Cloudflare Email Routing) must be untouched. Do not create
   wildcard records — GitHub's docs explicitly warn they enable domain takeover.

## Post-checks

- `dig runwaybar.applair.in CNAME` shows `nitishagar.github.io`.
- `curl -I https://runwaybar.applair.in` returns `200` with the page content.
- `https://nitishagar.github.io/runwaybar/` redirects to the custom domain (GitHub
  adds the redirect once the binding is verified).

## Why this shape

The upstream CodexBar site uses exactly this pattern (static `docs/` on GitHub
Pages behind Cloudflare, `codexbar.app`), and the research doc verified the
GitHub-side rules (custom domain before DNS, permanent TXT, no wildcards, CNAME
file ignored under custom workflows) against GitHub's documentation on 2026-09-23.
