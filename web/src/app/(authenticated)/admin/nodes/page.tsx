"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import type { Node, NodeUsage, RegisterNodeRequest } from "@/lib/types";
import { formatBytes, relativeTime } from "@/lib/format";

function UsageBar({ label, used, capacity }: { label: string; used: number; capacity: number }) {
  const pct = capacity > 0 ? Math.min((used / capacity) * 100, 100) : 0;
  const color = pct > 90 ? "bg-red-500" : pct > 70 ? "bg-yellow-500" : "bg-green-500";
  return (
    <div className="space-y-1">
      <div className="flex justify-between text-xs text-gray-400">
        <span>{label}</span>
        <span>{pct.toFixed(0)}%</span>
      </div>
      <div className="h-1.5 rounded-full bg-gray-700">
        <div className={`h-full rounded-full ${color}`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

const emptyForm: RegisterNodeRequest = {
  name: "",
  host_os: "macos",
  host_arch: "arm64",
  cpu_cores: 8,
  memory_bytes: 16 * 1024 ** 3,
  disk_bytes_total: 256 * 1024 ** 3,
};

export default function NodesPage() {
  const [nodes, setNodes] = useState<Node[]>([]);
  const [usages, setUsages] = useState<Record<string, NodeUsage>>({});
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<RegisterNodeRequest>({ ...emptyForm });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchNodes = useCallback(async () => {
    try {
      const data = await api.listNodes();
      setNodes(data);

      const usageMap: Record<string, NodeUsage> = {};
      await Promise.all(
        data
          .filter((n) => n.online)
          .map(async (n) => {
            try {
              const u = await api.getNodeUsage(n.id);
              usageMap[n.id] = u;
            } catch {
              // usage endpoint may not exist yet
            }
          }),
      );
      setUsages(usageMap);
    } catch (e) {
      console.error("Failed to fetch nodes", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchNodes();
    const interval = setInterval(fetchNodes, 10_000);
    return () => clearInterval(interval);
  }, [fetchNodes]);

  async function handleRegister(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await api.registerNode(form);
      setShowForm(false);
      setForm({ ...emptyForm });
      await fetchNodes();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to register node");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold text-gray-100">Nodes</h1>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
        >
          {showForm ? "Cancel" : "Register Node"}
        </button>
      </div>

      {showForm && (
        <form
          onSubmit={handleRegister}
          className="rounded-xl border border-gray-800 bg-gray-900 p-6 space-y-4"
        >
          <h2 className="text-lg font-medium text-gray-100">Register New Node</h2>
          {error && (
            <p className="text-sm text-red-400">{error}</p>
          )}
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Name</span>
              <input
                type="text"
                required
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none"
                placeholder="mac-mini-01"
              />
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Host OS</span>
              <select
                value={form.host_os}
                onChange={(e) => setForm({ ...form, host_os: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              >
                <option value="macos">macOS</option>
                <option value="linux">Linux</option>
              </select>
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Host Arch</span>
              <select
                value={form.host_arch}
                onChange={(e) => setForm({ ...form, host_arch: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              >
                <option value="arm64">arm64</option>
                <option value="x86_64">x86_64</option>
              </select>
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">CPU Cores</span>
              <input
                type="number"
                required
                min={1}
                value={form.cpu_cores}
                onChange={(e) => setForm({ ...form, cpu_cores: Number(e.target.value) })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Memory (GB)</span>
              <input
                type="number"
                required
                min={1}
                value={Math.round(form.memory_bytes / 1024 ** 3)}
                onChange={(e) => setForm({ ...form, memory_bytes: Number(e.target.value) * 1024 ** 3 })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Disk (GB)</span>
              <input
                type="number"
                required
                min={1}
                value={Math.round(form.disk_bytes_total / 1024 ** 3)}
                onChange={(e) => setForm({ ...form, disk_bytes_total: Number(e.target.value) * 1024 ** 3 })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
          </div>
          <div className="flex justify-end">
            <button
              type="submit"
              disabled={submitting}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-50 transition-colors"
            >
              {submitting ? "Registering..." : "Register"}
            </button>
          </div>
        </form>
      )}

      {loading ? (
        <div className="text-center text-gray-400 py-12">Loading nodes...</div>
      ) : nodes.length === 0 ? (
        <div className="text-center text-gray-400 py-12">No nodes registered yet.</div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {nodes.map((node) => {
            const usage = usages[node.id];
            return (
              <div
                key={node.id}
                className="rounded-xl border border-gray-800 bg-gray-900 p-5 space-y-4 hover:border-gray-700 transition-colors"
              >
                <div className="flex items-start justify-between">
                  <div>
                    <h3 className="text-base font-medium text-gray-100">{node.name ?? "Unnamed"}</h3>
                    <p className="text-xs text-gray-500 mt-0.5">
                      {node.host_os} / {node.host_arch}
                    </p>
                  </div>
                  <span className="flex items-center gap-1.5 text-xs">
                    <span
                      className={`inline-block h-2 w-2 rounded-full ${
                        node.online ? "bg-green-400" : "bg-red-400"
                      }`}
                    />
                    <span className={node.online ? "text-green-400" : "text-red-400"}>
                      {node.online ? "Online" : "Offline"}
                    </span>
                  </span>
                </div>

                <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
                  <span className="text-gray-500">CPU</span>
                  <span className="text-gray-300 text-right">{node.cpu_cores ?? "-"} cores</span>
                  <span className="text-gray-500">Memory</span>
                  <span className="text-gray-300 text-right">
                    {node.memory_bytes ? formatBytes(node.memory_bytes) : "-"}
                  </span>
                  <span className="text-gray-500">Disk</span>
                  <span className="text-gray-300 text-right">
                    {node.disk_bytes_total ? formatBytes(node.disk_bytes_total) : "-"}
                  </span>
                  <span className="text-gray-500">Heartbeat</span>
                  <span className="text-gray-300 text-right">
                    {node.last_heartbeat_at ? relativeTime(node.last_heartbeat_at) : "never"}
                  </span>
                </div>

                {usage && (
                  <div className="space-y-2 pt-2 border-t border-gray-800">
                    <UsageBar label="CPU" used={usage.vcpus_used} capacity={usage.vcpus_capacity} />
                    <UsageBar
                      label="Memory"
                      used={usage.memory_bytes_used}
                      capacity={usage.memory_bytes_capacity}
                    />
                    <UsageBar
                      label="Disk"
                      used={usage.disk_bytes_used}
                      capacity={usage.disk_bytes_capacity}
                    />
                    <div className="flex justify-between text-xs text-gray-400">
                      <span>Environments</span>
                      <span>
                        {usage.environment_count} / {usage.max_environments}
                      </span>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
