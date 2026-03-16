"use client";

import { useEffect, useState, useCallback } from "react";
import Link from "next/link";
import { api } from "@/lib/api";
import type { Environment, Image, Node, NodeUsage } from "@/lib/types";
import { formatBytes, relativeTime } from "@/lib/format";
import StateBadge from "@/components/StateBadge";

export default function DashboardPage() {
  const [environments, setEnvironments] = useState<Environment[]>([]);
  const [images, setImages] = useState<Image[]>([]);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [nodeUsages, setNodeUsages] = useState<Map<string, NodeUsage>>(new Map());
  const [loading, setLoading] = useState(true);

  const imageMap = new Map(images.map((img) => [img.id, img]));

  const fetchData = useCallback(async () => {
    try {
      const [envs, imgs, nds] = await Promise.all([
        api.listEnvironments(),
        api.listImages(),
        api.listNodes(),
      ]);
      setEnvironments(envs);
      setImages(imgs);
      setNodes(nds);
      // Fetch usage for online nodes
      const usages = new Map<string, NodeUsage>();
      await Promise.all(
        nds
          .filter((n) => n.online === 1)
          .map(async (n) => {
            try {
              const u = await api.getNodeUsage(n.id);
              usages.set(n.id, u);
            } catch {}
          }),
      );
      setNodeUsages(usages);
    } catch (err) {
      console.error("Failed to fetch dashboard data:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-zinc-400">Loading...</div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold text-zinc-100">Machines</h1>
        <Link
          href="/environments/new"
          className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 transition-colors"
        >
          New
        </Link>
      </div>

      {/* Nodes */}
      {nodes.filter((n) => n.online === 1).length > 0 && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {nodes
            .filter((n) => n.online === 1)
            .map((n) => {
              const usage = nodeUsages.get(n.id);
              return (
                <div
                  key={n.id}
                  className="rounded-xl border border-zinc-800 bg-zinc-900 p-5"
                >
                  <div className="flex items-center justify-between mb-3">
                    <span className="text-sm font-medium text-zinc-200">
                      {n.name ?? n.id.slice(0, 8)}
                    </span>
                    <span className="text-xs text-zinc-500">
                      {n.host_os}/{n.host_arch}
                    </span>
                  </div>
                  <div className="space-y-2">
                    <ResourceBar
                      label="CPU"
                      used={usage?.vcpus_used ?? 0}
                      total={n.cpu_cores ?? 0}
                      format={(v) => `${v} cores`}
                    />
                    <ResourceBar
                      label="Memory"
                      used={usage?.memory_bytes_used ?? 0}
                      total={n.memory_bytes ?? 0}
                      format={formatBytes}
                    />
                    <ResourceBar
                      label="Disk"
                      used={usage?.disk_bytes_used ?? 0}
                      total={n.disk_bytes_total ?? 0}
                      format={formatBytes}
                    />
                  </div>
                  {usage && (
                    <p className="mt-2 text-xs text-zinc-600">
                      {usage.environment_count} machine{usage.environment_count !== 1 ? "s" : ""}
                    </p>
                  )}
                </div>
              );
            })}
        </div>
      )}

      {environments.length === 0 ? (
        <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-12 text-center">
          <p className="text-zinc-400 text-sm">No machines yet.</p>
          <Link
            href="/environments/new"
            className="mt-4 inline-block text-sm text-indigo-400 hover:text-indigo-300"
          >
            Create your first machine
          </Link>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {environments.map((env) => {
            const image = env.image_id ? imageMap.get(env.image_id) : null;
            return (
              <Link
                key={env.id}
                href={`/environments/${env.id}`}
                className="group rounded-xl border border-zinc-800 bg-zinc-900 p-5 hover:border-zinc-700 hover:bg-zinc-800/50 transition-colors"
              >
                <div className="flex items-start justify-between">
                  <span className="text-sm text-zinc-300 truncate max-w-[60%]" title={env.id}>
                    {env.name ?? env.id.slice(0, 8)}
                  </span>
                  <StateBadge state={env.state} />
                </div>

                <p className="mt-2 text-sm text-zinc-400">
                  {image?.name ?? "Unknown image"}
                </p>

                <div className="mt-3 flex items-center gap-3 text-xs text-zinc-500">
                  <span>{env.vcpus ?? 0} vCPU</span>
                  <span className="text-zinc-700">/</span>
                  <span>{env.memory_bytes ? formatBytes(env.memory_bytes) : "0 B"} RAM</span>
                  <span className="text-zinc-700">/</span>
                  <span>{env.disk_bytes ? formatBytes(env.disk_bytes) : "0 B"}</span>
                </div>

                {env.created_at && (
                  <p className="mt-2 text-xs text-zinc-600">
                    Created {relativeTime(env.created_at)}
                  </p>
                )}
              </Link>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ResourceBar({
  label,
  used,
  total,
  format,
}: {
  label: string;
  used: number;
  total: number;
  format: (v: number) => string;
}) {
  const pct = total > 0 ? Math.min((used / total) * 100, 100) : 0;
  const color = pct > 80 ? "bg-red-500" : pct > 50 ? "bg-yellow-500" : "bg-green-500";
  return (
    <div>
      <div className="flex justify-between text-xs text-zinc-500 mb-0.5">
        <span>{label}</span>
        <span>
          {format(used)} / {format(total)}
        </span>
      </div>
      <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden">
        <div
          className={`h-full rounded-full ${color} transition-all`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}
