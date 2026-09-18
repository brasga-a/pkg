# Cloudflare Worker for `pkg.atlantic.sh`

This Cloudflare Worker powers the rootless installer and release routing for `pkg`.

## Endpoints

- `GET /install` or `GET /install.sh`: Serves the [`install.sh`](../../install.sh) script as `text/plain; charset=utf-8` with CDN caching.
- `GET /`:
  - If requested by CLI (`curl` / `wget`): Returns the installer script directly (`curl -fsSL https://pkg.atlantic.sh | sh`).
  - If opened in a browser: Serves the terminal-styled dark-mode landing page with 1-click install command copy.
- `GET /releases/latest.txt`: Returns the latest release tag (e.g., `v0.1.0-beta.1`).
- `GET /releases/<asset>` or `GET /releases/<version>/<asset>`: Redirects (302) to GitHub Releases for fast, free binary asset downloads.
- `GET /github`: Redirects to the GitHub repository.
- `GET /docs`: Redirects to documentation.

## Deployment

### Option 1: Direct Deploy from CLI

```bash
cd infra/cloudflare
npx wrangler login
npx wrangler deploy
```

### Option 2: Automated Deploy with GitHub Actions

Add the following GitHub Repository Secrets (`Settings -> Secrets and variables -> Actions`):
- `CLOUDFLARE_API_TOKEN` (Create from Cloudflare Dashboard with `Edit Cloudflare Workers` permission)
- `CLOUDFLARE_ACCOUNT_ID` (Found on your Cloudflare dashboard URL or Workers overview)

The workflow `.github/workflows/deploy-worker.yml` will automatically deploy changes on pushes to `main`.

## Connecting Domain (`pkg.atlantic.sh`)

In the Cloudflare Dashboard:
1. Go to **Workers & Pages** -> select **`pkg-atlantic-sh`**.
2. Click on **Settings** -> **Domains & Routes**.
3. Click **Add** -> **Custom Domain**.
4. Type `pkg.atlantic.sh` and click **Add Custom Domain**.
   *(Cloudflare will automatically provision SSL/TLS and route DNS traffic).*
