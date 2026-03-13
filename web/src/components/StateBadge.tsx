import { stateColor } from "@/lib/format";

interface StateBadgeProps {
  state: string | null;
}

export default function StateBadge({ state }: StateBadgeProps) {
  const colors = stateColor(state);

  return (
    <span
      className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${colors}`}
    >
      {state ?? "unknown"}
    </span>
  );
}
