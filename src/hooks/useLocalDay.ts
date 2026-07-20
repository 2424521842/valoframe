import { useEffect, useState } from "react";

/**
 * Returns the current local calendar day and advances it when the app crosses
 * midnight. Focus/visibility synchronization also covers a suspended laptop.
 */
export function useLocalDay(): Date {
  const [localDay, setLocalDay] = useState(() => startOfLocalDay(new Date()));

  useEffect(() => {
    let timer: number | undefined;

    const syncDay = () => {
      const nextDay = startOfLocalDay(new Date());
      setLocalDay((currentDay) =>
        currentDay.getTime() === nextDay.getTime() ? currentDay : nextDay,
      );
    };

    const scheduleNextMidnight = () => {
      if (timer !== undefined) window.clearTimeout(timer);

      const now = new Date();
      const nextMidnight = startOfLocalDay(now);
      nextMidnight.setDate(nextMidnight.getDate() + 1);
      timer = window.setTimeout(() => {
        syncDay();
        scheduleNextMidnight();
      }, Math.max(1, nextMidnight.getTime() - now.getTime()));
    };

    const syncAndReschedule = () => {
      syncDay();
      scheduleNextMidnight();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") syncAndReschedule();
    };

    scheduleNextMidnight();
    window.addEventListener("focus", syncAndReschedule);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener("focus", syncAndReschedule);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, []);

  return localDay;
}

function startOfLocalDay(value: Date): Date {
  const day = new Date(value);
  day.setHours(0, 0, 0, 0);
  return day;
}
