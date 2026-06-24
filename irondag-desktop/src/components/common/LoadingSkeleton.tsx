import React from "react";

type WidthVariant = "full" | "medium" | "short";

interface LoadingSkeletonProps {
  lines?: number;
  width?: WidthVariant | WidthVariant[];
  height?: string;
}

const widthMap: Record<WidthVariant, string> = {
  full: "100%",
  medium: "60%",
  short: "30%"
};

export const LoadingSkeleton: React.FC<LoadingSkeletonProps> = ({
  lines = 3,
  width = "full",
  height = "1rem"
}) => {
  const getWidth = (index: number): string => {
    if (typeof width === "string") {
      return widthMap[width] || width;
    }
    const variant = width[index % width.length];
    return widthMap[variant] || "100%";
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
      {Array.from({ length: lines }).map((_, index) => (
        <div
          key={index}
          style={{
            width: getWidth(index),
            height,
            background: "#2a2a3e",
            borderRadius: 6,
            animation: "skeleton-pulse 1.5s ease-in-out infinite",
            animationDelay: `${index * 0.1}s`
          }}
        />
      ))}
      <style>{`
        @keyframes skeleton-pulse {
          0%, 100% { opacity: 0.3; }
          50% { opacity: 0.6; }
        }
      `}</style>
    </div>
  );
};

export const CardSkeleton: React.FC<{ count?: number }> = ({ count = 1 }) => {
  return (
    <>
      {Array.from({ length: count }).map((_, index) => (
        <div
          key={index}
          style={{
            padding: "1.5rem",
            borderRadius: 16,
            background: "rgba(30, 41, 59, 0.7)",
            border: "1px solid rgba(99, 102, 241, 0.2)",
            marginBottom: "1.5rem"
          }}
        >
          <LoadingSkeleton lines={4} width={["medium", "full", "full", "short"]} />
        </div>
      ))}
    </>
  );
};

export const StatCardSkeleton: React.FC<{ count?: number }> = ({ count = 1 }) => {
  return (
    <div style={{ display: "grid", gridTemplateColumns: `repeat(auto-fit, minmax(180px, 1fr))`, gap: "1rem" }}>
      {Array.from({ length: count }).map((_, index) => (
        <div
          key={index}
          style={{
            padding: "1rem",
            borderRadius: 10,
            background: "rgba(99, 102, 241, 0.1)",
            border: "1px solid rgba(99, 102, 241, 0.2)"
          }}
        >
          <div
            style={{
              width: "40%",
              height: "2rem",
              background: "#2a2a3e",
              borderRadius: 4,
              marginBottom: "0.5rem",
              animation: "skeleton-pulse 1.5s ease-in-out infinite",
              animationDelay: `${index * 0.1}s`
            }}
          />
          <div
            style={{
              width: "60%",
              height: "0.9rem",
              background: "#2a2a3e",
              borderRadius: 4,
              animation: "skeleton-pulse 1.5s ease-in-out infinite",
              animationDelay: `${index * 0.15}s`
            }}
          />
        </div>
      ))}
      <style>{`
        @keyframes skeleton-pulse {
          0%, 100% { opacity: 0.3; }
          50% { opacity: 0.6; }
        }
      `}</style>
    </div>
  );
};

export const TableRowSkeleton: React.FC<{ rows?: number; columns?: number }> = ({ rows = 5, columns = 4 }) => {
  return (
    <>
      {Array.from({ length: rows }).map((_, rowIndex) => (
        <div
          key={rowIndex}
          style={{
            display: "grid",
            gridTemplateColumns: `repeat(${columns}, 1fr)`,
            gap: "1rem",
            padding: "0.75rem 1rem",
            marginBottom: "0.5rem",
            background: "rgba(30, 41, 59, 0.5)",
            borderRadius: 8
          }}
        >
          {Array.from({ length: columns }).map((_, colIndex) => (
            <div
              key={colIndex}
              style={{
                width: "80%",
                height: "0.9rem",
                background: "#2a2a3e",
                borderRadius: 4,
                animation: "skeleton-pulse 1.5s ease-in-out infinite",
                animationDelay: `${(rowIndex * columns + colIndex) * 0.05}s`
              }}
            />
          ))}
        </div>
      ))}
      <style>{`
        @keyframes skeleton-pulse {
          0%, 100% { opacity: 0.3; }
          50% { opacity: 0.6; }
        }
      `}</style>
    </>
  );
};
