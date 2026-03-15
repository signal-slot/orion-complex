"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { api } from "@/lib/api";
import type { Image, Node } from "@/lib/types";

const MEMORY_OPTIONS = [
  { label: "4 GB", value: 4 * 1024 * 1024 * 1024 },
  { label: "8 GB", value: 8 * 1024 * 1024 * 1024 },
  { label: "16 GB", value: 16 * 1024 * 1024 * 1024 },
  { label: "32 GB", value: 32 * 1024 * 1024 * 1024 },
];

const DISK_OPTIONS = [
  { label: "20 GB", value: 20 * 1024 * 1024 * 1024 },
  { label: "40 GB", value: 40 * 1024 * 1024 * 1024 },
  { label: "80 GB", value: 80 * 1024 * 1024 * 1024 },
  { label: "160 GB", value: 160 * 1024 * 1024 * 1024 },
];

export default function NewEnvironmentPage() {
  const router = useRouter();
  const [images, setImages] = useState<Image[]>([]);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [name, setName] = useState("");
  const [imageId, setImageId] = useState("");
  const [vcpus, setVcpus] = useState(4);
  const [memoryBytes, setMemoryBytes] = useState(MEMORY_OPTIONS[1].value);
  const [diskBytes, setDiskBytes] = useState(DISK_OPTIONS[1].value);
  const [ttlHours, setTtlHours] = useState("");

  useEffect(() => {
    async function fetchData() {
      try {
        const [imgs, nds] = await Promise.all([
          api.listImages(),
          api.listNodes(),
        ]);
        setImages(imgs);
        setNodes(nds);
        if (imgs.length > 0) {
          setImageId(imgs[0].id);
        }
      } catch (err) {
        console.error("Failed to fetch form data:", err);
      } finally {
        setLoading(false);
      }
    }
    fetchData();
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);

    try {
      const ttlSeconds = ttlHours ? parseFloat(ttlHours) * 3600 : undefined;
      const env = await api.createEnvironment({
        image_id: imageId,
        name: name.trim() || undefined,
        vcpus,
        memory_bytes: memoryBytes,
        disk_bytes: diskBytes,
        ttl_seconds: ttlSeconds,
      });
      router.push(`/environments/${env.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create environment");
    } finally {
      setSubmitting(false);
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-zinc-400">Loading...</div>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-lg space-y-6">
      <h1 className="text-2xl font-semibold text-zinc-100">New Environment</h1>

      {error && (
        <div className="rounded-lg border border-red-800 bg-red-900/30 px-4 py-3 text-sm text-red-400">
          {error}
        </div>
      )}

      <form onSubmit={handleSubmit} className="space-y-5">
        {/* Name */}
        <div>
          <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Name (optional)
          </label>
          <input
            id="name"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Auto-generated if empty"
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          />
        </div>

        {/* Image */}
        <div>
          <label htmlFor="image" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Image
          </label>
          <select
            id="image"
            value={imageId}
            onChange={(e) => setImageId(e.target.value)}
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          >
            {images.map((img) => (
              <option key={img.id} value={img.id}>
                {img.name ?? img.id.slice(0, 8)} ({img.guest_os}/{img.guest_arch})
              </option>
            ))}
          </select>
        </div>

        {/* vCPUs */}
        <div>
          <label htmlFor="vcpus" className="block text-sm font-medium text-zinc-300 mb-1.5">
            vCPUs
          </label>
          <input
            id="vcpus"
            type="number"
            min={1}
            max={64}
            value={vcpus}
            onChange={(e) => setVcpus(parseInt(e.target.value) || 1)}
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          />
        </div>

        {/* Memory */}
        <div>
          <label htmlFor="memory" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Memory
          </label>
          <select
            id="memory"
            value={memoryBytes}
            onChange={(e) => setMemoryBytes(Number(e.target.value))}
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          >
            {MEMORY_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        {/* Disk */}
        <div>
          <label htmlFor="disk" className="block text-sm font-medium text-zinc-300 mb-1.5">
            Disk
          </label>
          <select
            id="disk"
            value={diskBytes}
            onChange={(e) => setDiskBytes(Number(e.target.value))}
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          >
            {DISK_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        {/* TTL */}
        <div>
          <label htmlFor="ttl" className="block text-sm font-medium text-zinc-300 mb-1.5">
            TTL (hours, optional)
          </label>
          <input
            id="ttl"
            type="number"
            min={0}
            step={0.5}
            value={ttlHours}
            onChange={(e) => setTtlHours(e.target.value)}
            placeholder="No expiration"
            className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
          />
        </div>

        <button
          type="submit"
          disabled={submitting || !imageId}
          className="w-full rounded-lg bg-indigo-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {submitting ? "Creating..." : "Create Environment"}
        </button>
      </form>
    </div>
  );
}
