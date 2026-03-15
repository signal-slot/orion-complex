"use client";

import { useEffect, useState, useCallback } from "react";
import Link from "next/link";
import { api } from "@/lib/api";
import type { Environment, Image } from "@/lib/types";
import { formatBytes, relativeTime } from "@/lib/format";
import StateBadge from "@/components/StateBadge";

export default function DashboardPage() {
  const [environments, setEnvironments] = useState<Environment[]>([]);
  const [images, setImages] = useState<Image[]>([]);
  const [loading, setLoading] = useState(true);

  const imageMap = new Map(images.map((img) => [img.id, img]));

  const fetchData = useCallback(async () => {
    try {
      const [envs, imgs] = await Promise.all([
        api.listEnvironments(),
        api.listImages(),
      ]);
      setEnvironments(envs);
      setImages(imgs);
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
        <h1 className="text-2xl font-semibold text-zinc-100">Environments</h1>
        <Link
          href="/environments/new"
          className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 transition-colors"
        >
          New
        </Link>
      </div>

      {environments.length === 0 ? (
        <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-12 text-center">
          <p className="text-zinc-400 text-sm">No environments yet.</p>
          <Link
            href="/environments/new"
            className="mt-4 inline-block text-sm text-indigo-400 hover:text-indigo-300"
          >
            Create your first environment
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
