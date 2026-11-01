import type { NextConfig } from "next";

// Where the FastAPI backend listens (`uv run chess_coach` from the repo root).
const BACKEND = process.env.CHESS_COACH_BACKEND ?? "http://127.0.0.1:8000";

const nextConfig: NextConfig = {
  cacheComponents: true,
  partialPrefetching: true,

  // The browser talks only to Next; `/api/*` is proxied to the backend so
  // there is one origin and no CORS configuration to keep in step.
  async rewrites() {
    return [{ source: "/api/:path*", destination: `${BACKEND}/:path*` }];
  },

  experimental: {
    // The rewrite proxy gives up after 30s by default
    // (next/dist/server/lib/router-utils/proxy-request.js). A first review of
    // a 61-ply game took 22s at depth 16, so longer games would be cut off.
    // Five minutes is arbitrary headroom. XXX: the real fix is running
    // reviews as background jobs with progress; see SESSION.md.
    proxyTimeout: 300_000,
  },
};

export default nextConfig;
