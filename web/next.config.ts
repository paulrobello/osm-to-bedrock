import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: 'standalone',
  // SEC-010: Add HTTP security headers to every response from the Next.js frontend.
  async headers() {
    return [
      {
        source: "/(.*)",
        headers: [
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
