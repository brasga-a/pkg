/**
 * Cloudflare Worker for pkg.atlantic.sh
 * Serves the rootless installer script and routes binary releases.
 */

const FALLBACK_REPO = "brasga-a/pkg";
const FALLBACK_BRANCH = "main";

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const pathname = url.pathname;
    const userAgent = request.headers.get("user-agent") || "";
    const isCli = userAgent.startsWith("curl/") || userAgent.startsWith("Wget/") || userAgent.startsWith("HTTPie/");

    const repo = env.GITHUB_REPO || FALLBACK_REPO;
    const branch = env.DEFAULT_BRANCH || FALLBACK_BRANCH;

    // 1. Installer Script Route: /install, /install.sh
    if (pathname === "/install" || pathname === "/install.sh") {
      return handleInstallScript(repo, branch);
    }

    // 2. Root Route: /
    if (pathname === "/" || pathname === "") {
      // If requested from curl / wget, serve installer script directly: curl -fsSL https://pkg.atlantic.sh | sh
      if (isCli) {
        return handleInstallScript(repo, branch);
      }
      return handleLandingPage(repo);
    }

    // 3. Releases: latest.txt
    if (pathname === "/releases/latest.txt" || pathname === "/latest.txt") {
      return handleLatestVersion(repo);
    }

    // 4. Binary Release Downloads: /releases/...
    if (pathname.startsWith("/releases/")) {
      const parts = pathname.replace(/^\/releases\//, "").split("/").filter(Boolean);
      
      // Case A: /releases/latest/<asset>
      if (parts[0] === "latest" && parts.length > 1) {
        const asset = parts.slice(1).join("/");
        return Response.redirect(`https://github.com/${repo}/releases/latest/download/${asset}`, 302);
      }

      // Case B: /releases/<version>/<asset> (e.g. /releases/v0.1.0-beta.1/pkg-linux-x86_64.tar.gz)
      if (parts.length >= 2) {
        const version = parts[0];
        const asset = parts.slice(1).join("/");
        return Response.redirect(`https://github.com/${repo}/releases/download/${version}/${asset}`, 302);
      }

      // Case C: /releases/<asset> (defaults to latest release)
      if (parts.length === 1) {
        const asset = parts[0];
        return Response.redirect(`https://github.com/${repo}/releases/latest/download/${asset}`, 302);
      }
    }

    // 5. Convenience Redirects
    if (pathname === "/github" || pathname === "/repo") {
      return Response.redirect(`https://github.com/${repo}`, 302);
    }

    if (pathname === "/docs") {
      return Response.redirect(`https://github.com/${repo}/blob/${branch}/docs/README.md`, 302);
    }

    return new Response("Not Found", { status: 404 });
  }
};

/**
 * Fetch and stream the install.sh script with CDN caching
 */
async function handleInstallScript(repo, branch) {
  const upstreamUrl = `https://raw.githubusercontent.com/${repo}/${branch}/install.sh`;
  
  try {
    const upstreamRes = await fetch(upstreamUrl, {
      cf: {
        cacheEverything: true,
        cacheTtl: 300 // 5 minutes cache on edge
      }
    });

    if (!upstreamRes.ok) {
      return new Response(`# Error: Failed to fetch install script from ${upstreamUrl}\n# HTTP Status: ${upstreamRes.status}\n`, {
        status: 502,
        headers: { "Content-Type": "text/plain; charset=utf-8" }
      });
    }

    const scriptText = await upstreamRes.text();
    return new Response(scriptText, {
      status: 200,
      headers: {
        "Content-Type": "text/plain; charset=utf-8",
        "Cache-Control": "public, max-age=300, s-maxage=300",
        "X-Content-Type-Options": "nosniff",
        "Access-Control-Allow-Origin": "*"
      }
    });
  } catch (err) {
    return new Response(`# Error fetching install.sh: ${err.message}\n`, {
      status: 500,
      headers: { "Content-Type": "text/plain; charset=utf-8" }
    });
  }
}

/**
 * Fetch latest tag from GitHub API and return as text/plain
 */
async function handleLatestVersion(repo) {
  try {
    const apiUrl = `https://api.github.com/repos/${repo}/releases/latest`;
    const res = await fetch(apiUrl, {
      headers: { "User-Agent": "pkg-atlantic-sh-worker" },
      cf: { cacheEverything: true, cacheTtl: 600 }
    });

    if (!res.ok) {
      return new Response("0.1.0-beta.1\n", {
        headers: { "Content-Type": "text/plain; charset=utf-8" }
      });
    }

    const data = await res.json();
    const tag = data.tag_name || "latest";
    return new Response(`${tag}\n`, {
      headers: {
        "Content-Type": "text/plain; charset=utf-8",
        "Cache-Control": "public, max-age=600",
        "Access-Control-Allow-Origin": "*"
      }
    });
  } catch (_e) {
    return new Response("0.1.0-beta.1\n", {
      headers: { "Content-Type": "text/plain; charset=utf-8" }
    });
  }
}

/**
 * Minimalist, dark-mode terminal-style landing page for browser visitors
 */
function handleLandingPage(repo) {
  const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>pkg - Universal Rootless Package Manager for Linux & AI Agents</title>
  <meta name="description" content="A universal, cross-distribution, rootless package manager for Linux — built for Humans and AI Agents.">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700&family=Inter:wght@400;600;700&display=swap" rel="stylesheet">
  <style>
    :root {
      --bg: #090d16;
      --card-bg: #111726;
      --card-border: #1e293b;
      --text-main: #f8fafc;
      --text-muted: #94a3b8;
      --accent: #38bdf8;
      --accent-glow: rgba(56, 189, 248, 0.15);
      --green: #4ade80;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background-color: var(--bg);
      color: var(--text-main);
      font-family: 'Inter', sans-serif;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      padding: 2rem 1rem;
    }
    .container {
      max-width: 800px;
      width: 100%;
      display: flex;
      flex-direction: column;
      gap: 2rem;
    }
    .badge {
      display: inline-flex;
      align-items: center;
      gap: 0.5rem;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      padding: 0.35rem 0.85rem;
      border-radius: 9999px;
      font-size: 0.85rem;
      color: var(--accent);
      font-family: 'JetBrains Mono', monospace;
      width: fit-content;
    }
    .badge-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: var(--green);
      box-shadow: 0 0 8px var(--green);
    }
    h1 {
      font-size: 2.8rem;
      font-weight: 700;
      letter-spacing: -0.03em;
      line-height: 1.15;
    }
    h1 span {
      color: var(--accent);
      background: linear-gradient(135deg, #38bdf8 0%, #818cf8 100%);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
    }
    p.lead {
      color: var(--text-muted);
      font-size: 1.15rem;
      line-height: 1.6;
    }
    .install-card {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 12px;
      padding: 1.25rem 1.5rem;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 1rem;
      font-family: 'JetBrains Mono', monospace;
      font-size: 1.05rem;
      box-shadow: 0 10px 25px -5px rgba(0,0,0,0.4);
    }
    .install-cmd {
      color: var(--text-main);
      overflow-x: auto;
      white-space: nowrap;
      user-select: all;
    }
    .install-cmd span.prompt {
      color: var(--accent);
      margin-right: 0.5rem;
      user-select: none;
    }
    .copy-btn {
      background: #1e293b;
      border: 1px solid #334155;
      color: var(--text-main);
      border-radius: 6px;
      padding: 0.5rem 1rem;
      font-size: 0.85rem;
      font-family: inherit;
      cursor: pointer;
      transition: all 0.2s ease;
      white-space: nowrap;
    }
    .copy-btn:hover {
      background: #334155;
      border-color: var(--accent);
    }
    .features-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
      gap: 1.25rem;
    }
    .feature-item {
      background: rgba(17, 23, 38, 0.6);
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 1.25rem;
    }
    .feature-item h3 {
      font-size: 1rem;
      font-weight: 600;
      margin-bottom: 0.4rem;
      color: var(--text-main);
    }
    .feature-item p {
      font-size: 0.875rem;
      color: var(--text-muted);
      line-height: 1.5;
    }
    .links {
      display: flex;
      gap: 1rem;
      font-size: 0.95rem;
    }
    .links a {
      color: var(--text-muted);
      text-decoration: none;
      transition: color 0.2s;
    }
    .links a:hover {
      color: var(--accent);
    }
  </style>
</head>
<body>
  <div class="container">
    <div class="badge">
      <span class="badge-dot"></span>
      v0.1.0-beta.1 • Rootless & Universal
    </div>
    
    <div>
      <h1>Package management for <span>Linux & AI Agents</span>.</h1>
      <p class="lead" style="margin-top: 0.75rem;">
        No root permissions. No system pollution. Runs .deb, RPM, and Arch packages inside isolated, reproducible user-space stores.
      </p>
    </div>

    <div class="install-card">
      <div class="install-cmd">
        <span class="prompt">$</span>curl -fsSL https://pkg.atlantic.sh/install | sh
      </div>
      <button class="copy-btn" onclick="navigator.clipboard.writeText('curl -fsSL https://pkg.atlantic.sh/install | sh'); this.innerText='Copied!'; setTimeout(()=>this.innerText='Copy', 2000)">
        Copy
      </button>
    </div>

    <div class="features-grid">
      <div class="feature-item">
        <h3>100% Rootless</h3>
        <p>Installs directly into <code>~/.local/share/pkg</code> without <code>sudo</code> or native system DB contamination.</p>
      </div>
      <div class="feature-item">
        <h3>AI Agent Native</h3>
        <p>Integrated Model Context Protocol (MCP) server, structured JSON contracts, and non-interactive safeguards.</p>
      </div>
      <div class="feature-item">
        <h3>Atomic & Crash-Safe</h3>
        <p>Isolated store with zero-downtime activation, transactional generation switches, and automated recovery.</p>
      </div>
    </div>

    <div class="links">
      <a href="https://github.com/${repo}" target="_blank">GitHub Repository →</a>
      <a href="/docs">Documentation →</a>
      <a href="/releases/latest.txt">Latest Release →</a>
    </div>
  </div>
</body>
</html>`;

  return new Response(html, {
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "public, max-age=300"
    }
  });
}
