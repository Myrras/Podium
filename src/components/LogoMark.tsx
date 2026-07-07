/** Podium brand mark: a three-step winner's podium, drawn in `currentColor`. */

import type { SVGProps } from "react";

interface LogoMarkProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

export function LogoMark({ size = 18, ...props }: LogoMarkProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 100 100"
      width={size}
      height={size}
      {...props}
    >
      <defs>
        <filter id="lm-shadow" x="-40%" y="-20%" width="180%" height="140%">
          <feDropShadow
            dx="0"
            dy="2"
            stdDeviation="3"
            floodColor="currentColor"
            floodOpacity="0.18"
          />
        </filter>
      </defs>
      <g fill="currentColor" filter="url(#lm-shadow)">
        <rect x="8" y="52" width="26" height="38" rx="4" opacity="0.7" />
        <rect x="37" y="32" width="26" height="58" rx="4" />
        <rect x="66" y="62" width="26" height="28" rx="4" opacity="0.5" />
      </g>
    </svg>
  );
}
