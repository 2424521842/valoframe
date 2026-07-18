import { m, useReducedMotion } from "motion/react";
import { motionProfile } from "../lib/motionProfile";

export function AmbientBackdrop() {
  const profile = motionProfile(Boolean(useReducedMotion()));

  return (
    <div aria-hidden="true" className="ambient-backdrop">
      <m.span
        animate={
          profile.ambient
            ? { x: [0, 26, 0], y: [0, -18, 0] }
            : undefined
        }
        className="ambient-orb ambient-orb--violet"
        transition={{ duration: 18, ease: "easeInOut", repeat: Infinity }}
      />
      <m.span
        animate={
          profile.ambient
            ? { x: [0, -22, 0], y: [0, 14, 0] }
            : undefined
        }
        className="ambient-orb ambient-orb--mint"
        transition={{ duration: 22, ease: "easeInOut", repeat: Infinity }}
      />
      <span className="ambient-noise" />
    </div>
  );
}
