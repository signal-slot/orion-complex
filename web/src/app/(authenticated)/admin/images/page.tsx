"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import type { Image, RegisterImageRequest } from "@/lib/types";
import { formatDate } from "@/lib/format";

const emptyForm: RegisterImageRequest = {
  name: "",
  provider: "macos",
  guest_os: "",
  guest_arch: "arm64",
};

export default function ImagesPage() {
  const [images, setImages] = useState<Image[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<RegisterImageRequest>({ ...emptyForm });
  const [submitting, setSubmitting] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fetchImages = useCallback(async () => {
    try {
      const data = await api.listImages();
      setImages(data);
    } catch (e) {
      console.error("Failed to fetch images", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchImages();
  }, [fetchImages]);

  async function handleRegister(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await api.registerImage(form);
      setShowForm(false);
      setForm({ ...emptyForm });
      await fetchImages();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to register image");
    } finally {
      setSubmitting(false);
    }
  }

  async function handleDelete(id: string) {
    if (!confirm("Are you sure you want to delete this image?")) return;
    setDeleting(id);
    try {
      await api.deleteImage(id);
      await fetchImages();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to delete image");
    } finally {
      setDeleting(null);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold text-gray-100">Images</h1>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition-colors"
        >
          {showForm ? "Cancel" : "Register Image"}
        </button>
      </div>

      {showForm && (
        <form
          onSubmit={handleRegister}
          className="rounded-xl border border-gray-800 bg-gray-900 p-6 space-y-4"
        >
          <h2 className="text-lg font-medium text-gray-100">Register New Image</h2>
          {error && (
            <p className="text-sm text-red-400">{error}</p>
          )}
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Name</span>
              <input
                type="text"
                required
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none"
                placeholder="macOS 15 Sequoia"
              />
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Provider</span>
              <select
                value={form.provider}
                onChange={(e) => setForm({ ...form, provider: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              >
                <option value="macos">macOS</option>
                <option value="libvirt">libvirt</option>
              </select>
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Guest OS</span>
              <input
                type="text"
                required
                value={form.guest_os}
                onChange={(e) => setForm({ ...form, guest_os: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:border-blue-500 focus:outline-none"
                placeholder="macos"
              />
            </label>
            <label className="space-y-1">
              <span className="text-sm text-gray-400">Guest Arch</span>
              <select
                value={form.guest_arch}
                onChange={(e) => setForm({ ...form, guest_arch: e.target.value })}
                className="block w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-blue-500 focus:outline-none"
              >
                <option value="arm64">arm64</option>
                <option value="x86_64">x86_64</option>
              </select>
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
        <div className="text-center text-gray-400 py-12">Loading images...</div>
      ) : images.length === 0 ? (
        <div className="text-center text-gray-400 py-12">No images registered yet.</div>
      ) : (
        <div className="overflow-x-auto rounded-xl border border-gray-800">
          <table className="w-full text-sm text-left">
            <thead className="bg-gray-900 text-gray-400 text-xs uppercase tracking-wider">
              <tr>
                <th className="px-5 py-3">Name</th>
                <th className="px-5 py-3">Provider</th>
                <th className="px-5 py-3">Guest OS</th>
                <th className="px-5 py-3">Guest Arch</th>
                <th className="px-5 py-3">Created</th>
                <th className="px-5 py-3 text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-800">
              {images.map((img) => (
                <tr
                  key={img.id}
                  className="bg-gray-950 hover:bg-gray-900 transition-colors"
                >
                  <td className="px-5 py-3 font-medium text-gray-100">
                    {img.name ?? "Unnamed"}
                  </td>
                  <td className="px-5 py-3 text-gray-300">
                    <span className="inline-block rounded-md bg-gray-800 px-2 py-0.5 text-xs">
                      {img.provider}
                    </span>
                  </td>
                  <td className="px-5 py-3 text-gray-300">{img.guest_os ?? "-"}</td>
                  <td className="px-5 py-3 text-gray-300">{img.guest_arch ?? "-"}</td>
                  <td className="px-5 py-3 text-gray-400">
                    {img.created_at ? formatDate(img.created_at) : "-"}
                  </td>
                  <td className="px-5 py-3 text-right">
                    <button
                      onClick={() => handleDelete(img.id)}
                      disabled={deleting === img.id}
                      className="rounded-lg border border-red-800 px-3 py-1 text-xs text-red-400 hover:bg-red-900/30 disabled:opacity-50 transition-colors"
                    >
                      {deleting === img.id ? "Deleting..." : "Delete"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
