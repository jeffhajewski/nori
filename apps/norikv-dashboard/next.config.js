/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Disable server-side rendering for dashboard components
  // since they rely heavily on client-side WebSocket connections
  experimental: {
    // Enable React Server Components
  },
};

module.exports = nextConfig;
