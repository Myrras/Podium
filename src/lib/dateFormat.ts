/** Compact local-time label (e.g. "Apr 3, 14:05"); empty string for null. */
export function formatTime(iso: string | null): string {
  if (!iso) return "";
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Ordinal suffix for a day-of-month (1st, 2nd, 3rd, 4th, 11th, …). */
function ordinal(day: number): string {
  if (day >= 11 && day <= 13) return `${day}th`;
  switch (day % 10) {
    case 1:
      return `${day}st`;
    case 2:
      return `${day}nd`;
    case 3:
      return `${day}rd`;
    default:
      return `${day}th`;
  }
}

/**
 * Long local date/time with an ordinal day (e.g. "July 14th, 3:00 PM"); empty
 * string for null.
 */
export function formatLongTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  const month = d.toLocaleString(undefined, { month: "long" });
  const time = d.toLocaleString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  return `${month} ${ordinal(d.getDate())}, ${time}`;
}

/**
 * Short "time ago" label (e.g. "3 minutes ago", "just now"); empty string for
 * null. `compact` gives a terse age instead ("now", "3m", "2h", "5d") for
 * tight rows. `now` is injectable for tests.
 */
export function formatRelativeTime(
  iso: string | null,
  now: Date = new Date(),
  { compact = false }: { compact?: boolean } = {},
): string {
  if (!iso) return "";
  const deltaMs = now.getTime() - new Date(iso).getTime();
  const deltaS = Math.round(deltaMs / 1000);
  if (deltaS < 45) return compact ? "now" : "just now";
  const units: [number, string][] = [
    [60, "second"],
    [60, "minute"],
    [24, "hour"],
    [30, "day"],
    [12, "month"],
    [Number.POSITIVE_INFINITY, "year"],
  ];
  let value = deltaS;
  let unit = "second";
  for (const [size, name] of units) {
    if (value < size) {
      unit = name;
      break;
    }
    value = Math.round(value / size);
    unit = name;
  }
  if (compact) return `${value}${COMPACT_UNITS[unit]}`;
  return `${value} ${unit}${value === 1 ? "" : "s"} ago`;
}

const COMPACT_UNITS: Record<string, string> = {
  second: "s",
  minute: "m",
  hour: "h",
  day: "d",
  month: "mo",
  year: "y",
};

/**
 * Full "Updated" footer label combining the long date/time with a relative
 * suffix (e.g. "July 14th, 3:00 PM (3 minutes ago)"); empty string for null.
 */
export function formatUpdatedAt(
  iso: string | null,
  now: Date = new Date(),
): string {
  if (!iso) return "";
  return `${formatLongTime(iso)} (${formatRelativeTime(iso, now)})`;
}
