import { useEffect, useRef } from "react";
import { imageUrl } from "../shared/api";
import { renderPlate } from "../shared/plate";
import type { Painting, PlateSettings } from "../shared/types";

/** A painting thumbnail with a live plate rendered on top, like on the desktop. */
export function PlatePreview({
  painting,
  plate,
  scale = 0.62,
  className = "",
  showArt = true,
}: {
  painting: Painting;
  plate: PlateSettings;
  scale?: number;
  className?: string;
  showArt?: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    el.querySelectorAll(".plate").forEach((n) => n.remove());
    const p = renderPlate(painting, { ...plate, scale: plate.scale * scale });
    p.style.position = "absolute";
    const pos = plate.defaultPosition;
    const m = 14;
    el.append(p);
    const place = () => {
      const w = el.clientWidth - 2 * m - p.offsetWidth;
      const h = el.clientHeight - 2 * m - p.offsetHeight;
      p.style.left = `${m + Math.max(0, w) * pos.x}px`;
      p.style.top = `${m + Math.max(0, h) * pos.y}px`;
    };
    place();
    const ro = new ResizeObserver(place);
    ro.observe(el);
    return () => {
      ro.disconnect();
      p.remove();
    };
  }, [painting, plate, scale]);
  return (
    <div ref={host} className={`plate-preview ${className}`}>
      {showArt && <img src={imageUrl.thumb(painting.id)} alt="" />}
    </div>
  );
}
