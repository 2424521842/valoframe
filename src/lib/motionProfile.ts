export type MotionProfile = {
  enterY: number;
  hoverY: number;
  duration: number;
  stagger: number;
  ambient: boolean;
};

export function motionProfile(reducedMotion: boolean): MotionProfile {
  return reducedMotion
    ? {
        enterY: 0,
        hoverY: 0,
        duration: 0.01,
        stagger: 0,
        ambient: false,
      }
    : {
        enterY: 12,
        hoverY: -4,
        duration: 0.38,
        stagger: 0.035,
        ambient: true,
      };
}
