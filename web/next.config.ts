import type { NextConfig } from "next";

// SEC-003: Content-Security-Policy for the Next.js frontend.
//
// Browser-side external reach (verified by auditing src/components, src/hooks,
// and src/app/page.tsx): the ONLY cross-origin host the browser contacts is
// https://tile.openstreetmap.org (the OpenLayers `OSM` raster tile source,
// hard-coded via `new OSM()` — not user-configurable). Everything else
// (Overpass, Nominatim, the Rust API) is fetched server-side via the
// `/api/*` proxy routes (see src/app/api/* and src/lib/api-config.ts), so the
// browser only ever issues same-origin requests. In particular, the
// user-configurable Overpass URL flows to `/api/overpass` as a POST body and
// is then fetched server-side under an HTTPS allowlist — it never reaches the
// browser's network stack and therefore has no CSP impact.
//
// `script-src 'unsafe-inline'` and `style-src 'unsafe-inline'` are required
// because Next.js injects inline hydration/bootstrap scripts and OpenLayers /
// Next inject inline styles. A nonce-based policy would remove
// `'unsafe-inline'` but requires a `proxy.ts` + dynamic-rendering migration,
// which is out of scope for this surgical fix; see
// node_modules/next/dist/docs/01-app/02-guides/content-security-policy.md.
// `'unsafe-eval'` is added only in dev because React uses eval for debug info.
const isDev = process.env.NODE_ENV === 'development';
const cspHeader = [
  "default-src 'self'",
  `script-src 'self' 'unsafe-inline'${isDev ? " 'unsafe-eval'" : ''}`,
  // OpenLayers + Next inject inline styles; nonces not used here.
  "style-src 'self' 'unsafe-inline'",
  // OSM raster tiles + inline SVG icons + OL blob image sources.
  "img-src 'self' data: blob: https://tile.openstreetmap.org",
  // All backend calls go through same-origin /api/* proxies.
  "connect-src 'self'",
  "font-src 'self'",
  "object-src 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "frame-ancestors 'none'",
  "upgrade-insecure-requests",
].join('; ');

const nextConfig: NextConfig = {
  output: 'standalone',
  // SEC-010 / SEC-003: Add HTTP security headers (incl. CSP) to every response.
  async headers() {
    return [
      {
        source: "/(.*)",
        headers: [
          // SEC-003: restrictive CSP. CSP frame-ancestors supersedes
          // X-Frame-Options in modern browsers, so the directive below
          // effectively makes the page non-frameable.
          { key: "Content-Security-Policy", value: cspHeader },
          // Prevent the page from being embedded in an iframe on other origins.
          { key: "X-Frame-Options", value: "SAMEORIGIN" },
          // Prevent MIME-type sniffing of responses.
          { key: "X-Content-Type-Options", value: "nosniff" },
          // Do not send the full referrer URL to cross-origin destinations.
          { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
          // Opt out of interest-cohort FLoC / Topics API tracking.
          { key: "Permissions-Policy", value: "interest-cohort=()" },
        ],
      },
    ];
  },
};

export default nextConfig;
