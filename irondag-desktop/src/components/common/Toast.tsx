import React, { useEffect, useState } from "react";

export type ToastType = "success" | "error" | "warning" | "info";

export interface ToastData {
  id: string;
  message: string;
  type: ToastType;
  duration?: number;
}

const toastColors: Record<ToastType, { bg: string; border: string; text: string; icon: string }> = {
  success: { bg: "rgba(16, 185, 129, 0.15)", border: "rgba(16, 185, 129, 0.4)", text: "#10b981", icon: "✓" },
  error: { bg: "rgba(239, 68, 68, 0.15)", border: "rgba(239, 68, 68, 0.4)", text: "#ef4444", icon: "✕" },
  warning: { bg: "rgba(245, 158, 11, 0.15)", border: "rgba(245, 158, 11, 0.4)", text: "#f59e0b", icon: "⚠" },
  info: { bg: "rgba(99, 102, 241, 0.15)", border: "rgba(99, 102, 241, 0.4)", text: "#6366f1", icon: "ℹ" }
};

interface ToastProps {
  toast: ToastData;
  onDismiss: (id: string) => void;
}

export const Toast: React.FC<ToastProps> = ({ toast, onDismiss }) => {
  const [isExiting, setIsExiting] = useState(false);
  const colors = toastColors[toast.type];

  useEffect(() => {
    const duration = toast.duration ?? 5000;
    const timer = setTimeout(() => {
      setIsExiting(true);
      setTimeout(() => onDismiss(toast.id), 300);
    }, duration);

    return () => clearTimeout(timer);
  }, [toast.id, toast.duration, onDismiss]);

  const handleDismiss = () => {
    setIsExiting(true);
    setTimeout(() => onDismiss(toast.id), 300);
  };

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "0.75rem",
        padding: "0.875rem 1rem",
        background: colors.bg,
        border: `1px solid ${colors.border}`,
        borderRadius: 10,
        boxShadow: "0 4px 12px rgba(0, 0, 0, 0.3)",
        backdropFilter: "blur(12px)",
        animation: isExiting ? "toast-exit 0.3s ease-out forwards" : "toast-enter 0.3s ease-out",
        marginBottom: "0.5rem"
      }}
    >
      <div
        style={{
          width: "24px",
          height: "24px",
          borderRadius: "50%",
          background: colors.text,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontSize: "0.75rem",
          color: "white",
          fontWeight: "700",
          flexShrink: 0
        }}
      >
        {colors.icon}
      </div>
      <span
        style={{
          flex: 1,
          color: "#f8fafc",
          fontSize: "0.9rem",
          fontWeight: "500"
        }}
      >
        {toast.message}
      </span>
      <button
        onClick={handleDismiss}
        style={{
          background: "transparent",
          border: "none",
          color: "#94a3b8",
          cursor: "pointer",
          padding: "0.25rem",
          fontSize: "1rem",
          lineHeight: 1,
          transition: "color 0.2s ease"
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.color = "#f8fafc";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.color = "#94a3b8";
        }}
      >
        ✕
      </button>
      <style>{`
        @keyframes toast-enter {
          from {
            opacity: 0;
            transform: translateX(100%);
          }
          to {
            opacity: 1;
            transform: translateX(0);
          }
        }
        @keyframes toast-exit {
          from {
            opacity: 1;
            transform: translateX(0);
          }
          to {
            opacity: 0;
            transform: translateX(100%);
          }
        }
      `}</style>
    </div>
  );
};

interface ToastContainerProps {
  toasts: ToastData[];
  onDismiss: (id: string) => void;
}

export const ToastContainer: React.FC<ToastContainerProps> = ({ toasts, onDismiss }) => {
  return (
    <div
      style={{
        position: "fixed",
        top: "1.5rem",
        right: "1.5rem",
        zIndex: 9999,
        maxWidth: "380px",
        width: "100%"
      }}
    >
      {toasts.map((toast) => (
        <Toast key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
};
