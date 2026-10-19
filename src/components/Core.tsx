import { useEffect, useRef } from "react";
import type { CoreState } from "../lib/conversation";
import { useReducedMotion } from "../lib/useReducedMotion";

const PALETTE: Record<CoreState, { hue: string; glow: string; speed: number; pulse: number }> = {
  idle: { hue: "56, 225, 255", glow: "0.35", speed: 0.25, pulse: 0.04 },
  thinking: { hue: "140, 160, 255", glow: "0.55", speed: 1.1, pulse: 0.1 },
  responding: { hue: "56, 225, 255", glow: "0.75", speed: 0.7, pulse: 0.16 },
  error: { hue: "255, 110, 90", glow: "0.5", speed: 0.15, pulse: 0.06 },
  offline: { hue: "120, 135, 150", glow: "0.18", speed: 0, pulse: 0 },
};

const RINGS = [
  { r: 0.92, width: 1.5, segments: 3, gap: 0.5, dir: 1 },
  { r: 0.78, width: 3, segments: 6, gap: 0.35, dir: -1.4 },
  { r: 0.64, width: 1, segments: 1, gap: 0.12, dir: 0.6 },
  { r: 0.52, width: 2, segments: 12, gap: 0.6, dir: -0.8 },
];

interface Props {
  state: CoreState;
  size?: number;
}

/** Animated HUD core. Its motion is driven only by the real activity state. */
export function Core({ state, size = 220 }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  const reduced = useReducedMotion();

  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size * dpr;
    canvas.height = size * dpr;
    ctx.scale(dpr, dpr);
    const p = PALETTE[state];
    const c = size / 2;
    let frame = 0;
    const start = performance.now();

    const draw = (now: number) => {
      const t = reduced ? 0 : (now - start) / 1000;
      ctx.clearRect(0, 0, size, size);

      const pulse = 1 + Math.sin(t * 3) * p.pulse;
      const grad = ctx.createRadialGradient(c, c, 0, c, c, c * 0.42 * pulse);
      grad.addColorStop(0, `rgba(${p.hue}, 0.95)`);
      grad.addColorStop(0.35, `rgba(${p.hue}, ${p.glow})`);
      grad.addColorStop(1, `rgba(${p.hue}, 0)`);
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.arc(c, c, c * 0.42 * pulse, 0, Math.PI * 2);
      ctx.fill();

      for (const ring of RINGS) {
        const rot = t * p.speed * ring.dir;
        const seg = (Math.PI * 2) / ring.segments;
        ctx.strokeStyle = `rgba(${p.hue}, 0.8)`;
        ctx.lineWidth = ring.width;
        ctx.lineCap = "round";
        for (let i = 0; i < ring.segments; i++) {
          const a0 = rot + i * seg;
          ctx.beginPath();
          ctx.arc(c, c, c * ring.r, a0, a0 + seg * (1 - ring.gap));
          ctx.stroke();
        }
      }

      ctx.strokeStyle = `rgba(${p.hue}, 0.25)`;
      ctx.lineWidth = 1;
      for (let i = 0; i < 60; i++) {
        const a = (i / 60) * Math.PI * 2;
        const inner = i % 5 === 0 ? 0.97 : 0.985;
        ctx.beginPath();
        ctx.moveTo(c + Math.cos(a) * c * inner, c + Math.sin(a) * c * inner);
        ctx.lineTo(c + Math.cos(a) * c, c + Math.sin(a) * c);
        ctx.stroke();
      }

      if (!reduced && p.speed > 0) frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [state, size, reduced]);

  return <canvas ref={ref} className={`core core--${state}`} style={{ width: size, height: size }} aria-hidden="true" />;
}
