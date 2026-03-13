"use client";

import { useEffect, useState, useCallback } from "react";
import { api } from "@/lib/api";
import type { UserSshKey } from "@/lib/types";
import { relativeTime } from "@/lib/format";

export default function SshKeysPage() {
  const [keys, setKeys] = useState<UserSshKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);

  const fetchKeys = useCallback(async () => {
    try {
      const data = await api.listSshKeys();
      setKeys(data);
    } catch (err) {
      console.error("Failed to fetch SSH keys:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchKeys();
  }, [fetchKeys]);

  async function handleAdd(e: React.FormEvent) {
    e.preventDefault();
    if (!publicKey.trim()) return;
    setAdding(true);
    setError(null);

    try {
      await api.addSshKey({ public_key: publicKey.trim() });
      setName("");
      setPublicKey("");
      await fetchKeys();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to add SSH key");
    } finally {
      setAdding(false);
    }
  }

  async function handleDelete(id: string) {
    setDeleting(id);
    try {
      await api.deleteSshKey(id);
      setDeleteConfirmId(null);
      await fetchKeys();
    } catch (err) {
      console.error("Failed to delete SSH key:", err);
    } finally {
      setDeleting(null);
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
    <div className="space-y-8 max-w-2xl">
      <h1 className="text-2xl font-semibold text-zinc-100">SSH Keys</h1>

      {/* Add Key Form */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <h2 className="text-sm font-medium text-zinc-400 mb-4">Add SSH Key</h2>

        {error && (
          <div className="mb-4 rounded-lg border border-red-800 bg-red-900/30 px-4 py-3 text-sm text-red-400">
            {error}
          </div>
        )}

        <form onSubmit={handleAdd} className="space-y-4">
          <div>
            <label htmlFor="key-name" className="block text-sm font-medium text-zinc-300 mb-1.5">
              Name
            </label>
            <input
              id="key-name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My laptop"
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
            />
          </div>

          <div>
            <label htmlFor="public-key" className="block text-sm font-medium text-zinc-300 mb-1.5">
              Public Key
            </label>
            <textarea
              id="public-key"
              rows={3}
              value={publicKey}
              onChange={(e) => setPublicKey(e.target.value)}
              placeholder="ssh-ed25519 AAAA..."
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 font-mono focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500 resize-none"
            />
          </div>

          <button
            type="submit"
            disabled={adding || !publicKey.trim()}
            className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {adding ? "Adding..." : "Add Key"}
          </button>
        </form>
      </div>

      {/* Key List */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <h2 className="text-sm font-medium text-zinc-400 mb-4">Your Keys</h2>

        {keys.length === 0 ? (
          <p className="text-sm text-zinc-500">No SSH keys added yet.</p>
        ) : (
          <ul className="divide-y divide-zinc-800">
            {keys.map((key) => (
              <li key={key.id} className="flex items-center justify-between py-4">
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-mono text-zinc-300 truncate">
                    {key.fingerprint ?? key.public_key?.slice(0, 40) ?? key.id.slice(0, 8)}
                  </p>
                  {key.created_at && (
                    <p className="mt-1 text-xs text-zinc-500">
                      Added {relativeTime(key.created_at)}
                    </p>
                  )}
                </div>

                <div className="ml-4 flex-shrink-0">
                  {deleteConfirmId === key.id ? (
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => handleDelete(key.id)}
                        disabled={deleting === key.id}
                        className="rounded-lg bg-red-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-red-500 disabled:opacity-50 transition-colors"
                      >
                        {deleting === key.id ? "Deleting..." : "Confirm"}
                      </button>
                      <button
                        onClick={() => setDeleteConfirmId(null)}
                        className="rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => setDeleteConfirmId(key.id)}
                      className="rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-xs text-red-400 hover:bg-red-900/30 hover:border-red-700 transition-colors"
                    >
                      Delete
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
