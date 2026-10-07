/**
 * Thin REST helpers against the Rust API. In dev, Vite proxies /api -> localhost:3000.
 * gRPC-web / Connect bindings generated from packages/proto will live alongside this later.
 */

const BASE = import.meta.env.VITE_API_BASE ?? '/api';

export interface Health {
  status: string;
  version: string;
}

export async function fetchHealth(): Promise<Health> {
  const res = await fetch(`${BASE}/health`);
  if (!res.ok) throw new Error(`health check failed: ${res.status}`);
  return res.json() as Promise<Health>;
}
