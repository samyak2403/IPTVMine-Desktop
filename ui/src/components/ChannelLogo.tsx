import { useEffect, useState } from "react";
import { Tv } from "lucide-react";

interface Props {
  src?: string | null;
  name: string;
  size?: number;
  hideOnMissing?: boolean;
}

export function ChannelLogo({ src, name, size = 44, hideOnMissing = false }: Props) {
  const [failed, setFailed] = useState(false);
  const cleanSrc = (src ?? "").trim();

  useEffect(() => {
    setFailed(false);
  }, [cleanSrc]);

  if ((!cleanSrc || failed) && hideOnMissing) {
    return null;
  }

  const initials = name
    .replace(/[^a-zA-Z0-9\s]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");

  return (
    <div
      className="channel-logo-box"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      {cleanSrc && !failed ? (
        <img
          className="channel-logo-img"
          src={cleanSrc}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      ) : initials ? (
        <span className="channel-logo-fallback">{initials}</span>
      ) : (
        <Tv size={Math.round(size * 0.44)} color="#ffc062" />
      )}
    </div>
  );
}
