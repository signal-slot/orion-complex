"use client";

import { useEffect, useState, useCallback } from "react";
import { useParams, useRouter } from "next/navigation";
import { api } from "@/lib/api";
import type { Environment, Image, Node, Snapshot, SshEndpoint } from "@/lib/types";
import { formatBytes, formatDate, relativeTime } from "@/lib/format";
import StateBadge from "@/components/StateBadge";

const TRANSIENT_STATES = new Set([
  "creating",
  "suspending",
  "resuming",
  "rebooting",
  "migrating",
  "destroying",
]);

export default function EnvironmentDetailPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const envId = params.id;

  const [env, setEnv] = useState<Environment | null>(null);
  const [image, setImage] = useState<Image | null>(null);
  const [node, setNode] = useState<Node | null>(null);
  const [sshEndpoint, setSshEndpoint] = useState<SshEndpoint | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [showDestroyModal, setShowDestroyModal] = useState(false);
  const [snapshotName, setSnapshotName] = useState("");
  const [creatingSnapshot, setCreatingSnapshot] = useState(false);

  const fetchEnvironment = useCallback(async () => {
    try {
      const envData = await api.getEnvironment(envId);
      setEnv(envData);

      if (envData.image_id && !image) {
        api.getImage(envData.image_id).then(setImage).catch(() => {});
      }
      if (envData.node_id && !node) {
        api.getNode(envData.node_id).then(setNode).catch(() => {});
      }
      if (envData.state === "running") {
        api.getSshEndpoint(envId).then(setSshEndpoint).catch(() => setSshEndpoint(null));
      }

      api.listSnapshots(envId).then(setSnapshots).catch(() => {});
    } catch (err) {
      console.error("Failed to fetch environment:", err);
    } finally {
      setLoading(false);
    }
  }, [envId, image, node]);

  useEffect(() => {
    fetchEnvironment();
  }, [fetchEnvironment]);

  useEffect(() => {
    if (!env || !TRANSIENT_STATES.has(env.state ?? "")) return;
    const interval = setInterval(fetchEnvironment, 3000);
    return () => clearInterval(interval);
  }, [env, fetchEnvironment]);

  async function handleAction(action: string) {
    if (!env) return;
    setActionLoading(action);
    try {
      switch (action) {
        case "suspend":
          await api.suspendEnvironment(envId);
          break;
        case "resume":
          await api.resumeEnvironment(envId);
          break;
        case "reboot":
          await api.rebootEnvironment(envId);
          break;
        case "destroy":
          await api.destroyEnvironment(envId);
          setShowDestroyModal(false);
          break;
      }
      await fetchEnvironment();
    } catch (err) {
      console.error(`Failed to ${action}:`, err);
    } finally {
      setActionLoading(null);
    }
  }

  async function handleCreateSnapshot() {
    setCreatingSnapshot(true);
    try {
      await api.createSnapshot(envId, { name: snapshotName || undefined });
      setSnapshotName("");
      const snaps = await api.listSnapshots(envId);
      setSnapshots(snaps);
    } catch (err) {
      console.error("Failed to create snapshot:", err);
    } finally {
      setCreatingSnapshot(false);
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-zinc-400">Loading...</div>
      </div>
    );
  }

  if (!env) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-zinc-400">Environment not found.</div>
      </div>
    );
  }

  const state = env.state ?? "";
  const isTransient = TRANSIENT_STATES.has(state);

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <button
            onClick={() => router.push("/dashboard")}
            className="text-zinc-500 hover:text-zinc-300 transition-colors"
          >
            &larr;
          </button>
          <h1 className="font-mono text-xl text-zinc-100">{env.id.slice(0, 12)}</h1>
          <StateBadge state={env.state} />
        </div>
      </div>

      {/* Info Grid */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <h2 className="text-sm font-medium text-zinc-400 mb-4">Details</h2>
        <dl className="grid grid-cols-2 gap-x-8 gap-y-4 sm:grid-cols-3 lg:grid-cols-4">
          <div>
            <dt className="text-xs text-zinc-500">Image</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{image?.name ?? env.image_id?.slice(0, 8) ?? "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Node</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{node?.name ?? env.node_id?.slice(0, 8) ?? "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">vCPUs</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{env.vcpus ?? "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Memory</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{env.memory_bytes ? formatBytes(env.memory_bytes) : "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Disk</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{env.disk_bytes ? formatBytes(env.disk_bytes) : "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Created</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">{env.created_at ? formatDate(env.created_at) : "-"}</dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Expires</dt>
            <dd className="mt-0.5 text-sm text-zinc-200">
              {env.expires_at ? relativeTime(env.expires_at) : "Never"}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-zinc-500">Full ID</dt>
            <dd className="mt-0.5 text-sm text-zinc-200 font-mono break-all">{env.id}</dd>
          </div>
        </dl>
      </div>

      {/* Access Section */}
      {state === "running" && (
        <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
          <h2 className="text-sm font-medium text-zinc-400 mb-3">Access</h2>
          <div className="space-y-3">
            {sshEndpoint && (() => {
              const vmHost = `orion-${env.id.slice(0, 8)}.local`;
              return (
                <div>
                  <p className="text-xs text-zinc-500 mb-1.5">SSH</p>
                  <div className="flex items-center gap-3">
                    <code className="flex-1 rounded-lg bg-zinc-800 px-4 py-2.5 text-sm text-green-400 font-mono">
                      ssh root@{vmHost}
                    </code>
                    <button
                      onClick={() => navigator.clipboard.writeText(`ssh root@${vmHost}`)}
                      className="rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors"
                    >
                      Copy
                    </button>
                  </div>
                </div>
              );
            })()}
            {env.guest_os === "macos" && (
              <div>
                <p className="text-xs text-zinc-500 mb-1.5">Screen Sharing (VNC)</p>
                <div className="flex items-center gap-3">
                  <code className="flex-1 rounded-lg bg-zinc-800 px-4 py-2.5 text-sm text-blue-400 font-mono">
                    vnc://orion-{env.id.slice(0, 8)}.local
                  </code>
                  <button
                    onClick={() => navigator.clipboard.writeText(`vnc://orion-${env.id.slice(0, 8)}.local`)}
                    className="rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2.5 text-xs text-zinc-300 hover:bg-zinc-700 transition-colors"
                  >
                    Copy
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Actions */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <h2 className="text-sm font-medium text-zinc-400 mb-3">Actions</h2>
        <div className="flex flex-wrap gap-3">
          {state === "running" && (
            <>
              <ActionButton
                label="Suspend"
                onClick={() => handleAction("suspend")}
                loading={actionLoading === "suspend"}
                className="border-yellow-700 text-yellow-400 hover:bg-yellow-900/30"
              />
              <ActionButton
                label="Reboot"
                onClick={() => handleAction("reboot")}
                loading={actionLoading === "reboot"}
                className="border-blue-700 text-blue-400 hover:bg-blue-900/30"
              />
            </>
          )}
          {state === "suspended" && (
            <ActionButton
              label="Resume"
              onClick={() => handleAction("resume")}
              loading={actionLoading === "resume"}
              className="border-green-700 text-green-400 hover:bg-green-900/30"
            />
          )}
          {(state === "running" || state === "suspended" || state === "failed" || isTransient) && (
            <ActionButton
              label="Destroy"
              onClick={() => setShowDestroyModal(true)}
              loading={false}
              className="border-red-700 text-red-400 hover:bg-red-900/30"
            />
          )}
        </div>
      </div>

      {/* Snapshots */}
      <div className="rounded-xl border border-zinc-800 bg-zinc-900 p-6">
        <h2 className="text-sm font-medium text-zinc-400 mb-4">Snapshots</h2>

        {state === "running" && (
          <div className="flex gap-3 mb-4">
            <input
              type="text"
              value={snapshotName}
              onChange={(e) => setSnapshotName(e.target.value)}
              placeholder="Snapshot name (optional)"
              className="flex-1 rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-200 placeholder-zinc-500 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
            />
            <button
              onClick={handleCreateSnapshot}
              disabled={creatingSnapshot}
              className="rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50 transition-colors"
            >
              {creatingSnapshot ? "Creating..." : "Create Snapshot"}
            </button>
          </div>
        )}

        {snapshots.length === 0 ? (
          <p className="text-sm text-zinc-500">No snapshots yet.</p>
        ) : (
          <ul className="divide-y divide-zinc-800">
            {snapshots.map((snap) => (
              <li key={snap.id} className="flex items-center justify-between py-3">
                <div>
                  <span className="text-sm text-zinc-200">
                    {snap.name ?? snap.id.slice(0, 8)}
                  </span>
                  {snap.created_at && (
                    <span className="ml-3 text-xs text-zinc-500">
                      {relativeTime(snap.created_at)}
                    </span>
                  )}
                </div>
                <span className="font-mono text-xs text-zinc-600">{snap.id.slice(0, 8)}</span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Destroy Confirmation Modal */}
      {showDestroyModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-6 w-full max-w-md shadow-xl">
            <h3 className="text-lg font-semibold text-zinc-100">Destroy Environment</h3>
            <p className="mt-2 text-sm text-zinc-400">
              Are you sure you want to destroy this environment? This action cannot be undone.
            </p>
            <div className="mt-6 flex justify-end gap-3">
              <button
                onClick={() => setShowDestroyModal(false)}
                className="rounded-lg border border-zinc-700 bg-zinc-800 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-700 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => handleAction("destroy")}
                disabled={actionLoading === "destroy"}
                className="rounded-lg bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-500 disabled:opacity-50 transition-colors"
              >
                {actionLoading === "destroy" ? "Destroying..." : "Destroy"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function ActionButton({
  label,
  onClick,
  loading,
  className,
}: {
  label: string;
  onClick: () => void;
  loading: boolean;
  className: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={loading}
      className={`rounded-lg border bg-transparent px-4 py-2 text-sm font-medium disabled:opacity-50 transition-colors ${className}`}
    >
      {loading ? `${label}...` : label}
    </button>
  );
}
