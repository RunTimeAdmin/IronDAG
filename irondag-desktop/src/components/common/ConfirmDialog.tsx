import React, { useEffect, useRef, useCallback } from "react";

interface ConfirmDialogProps {
  title: string;
  message: string;
  isOpen: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  confirmText?: string;
  cancelText?: string;
}

export const ConfirmDialog: React.FC<ConfirmDialogProps> = ({
  title,
  message,
  isOpen,
  onConfirm,
  onCancel,
  confirmText = "Confirm",
  cancelText = "Cancel"
}) => {
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelButtonRef = useRef<HTMLButtonElement>(null);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);
  const previouslyFocusedElement = useRef<HTMLElement | null>(null);

  // Store the previously focused element when dialog opens
  useEffect(() => {
    if (isOpen) {
      previouslyFocusedElement.current = document.activeElement as HTMLElement;
      // Focus the cancel button when dialog opens
      setTimeout(() => {
        cancelButtonRef.current?.focus();
      }, 0);
    } else if (previouslyFocusedElement.current) {
      // Return focus to the element that triggered the dialog
      previouslyFocusedElement.current.focus();
    }
  }, [isOpen]);

  // Handle keyboard events (Escape to close, Tab to trap focus)
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onCancel();
      return;
    }

    if (e.key === 'Tab') {
      const focusableElements = dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      );
      
      if (!focusableElements || focusableElements.length === 0) return;

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];

      if (e.shiftKey) {
        // Shift + Tab: move backwards
        if (document.activeElement === firstElement) {
          e.preventDefault();
          lastElement.focus();
        }
      } else {
        // Tab: move forwards
        if (document.activeElement === lastElement) {
          e.preventDefault();
          firstElement.focus();
        }
      }
    }
  }, [onCancel]);

  if (!isOpen) return null;

  return (
    <div 
      className="confirm-overlay" 
      onClick={onCancel}
      role="presentation"
      style={{
        position: "fixed",
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        background: "rgba(0,0,0,0.7)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
        backdropFilter: "blur(4px)"
      }}
    >
      <div 
        ref={dialogRef}
        className="confirm-dialog" 
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        style={{
          background: "#1e1e2e",
          border: "1px solid rgba(99,102,241,0.3)",
          borderRadius: 12,
          padding: 24,
          maxWidth: 420,
          width: "90%",
          boxShadow: "0 20px 60px rgba(0,0,0,0.5)"
        }}
      >
        <h3 id="confirm-dialog-title" style={{ color: "#e2e8f0", margin: "0 0 12px" }}>{title}</h3>
        <p style={{ color: "#94a3b8", margin: "0 0 20px", lineHeight: 1.5 }}>{message}</p>
        <div style={{ display: "flex", gap: 12, justifyContent: "flex-end" }}>
          <button
            ref={cancelButtonRef}
            onClick={onCancel}
            aria-label={cancelText}
            style={{
              padding: "8px 20px",
              borderRadius: 8,
              border: "1px solid #475569",
              background: "transparent",
              color: "#94a3b8",
              cursor: "pointer"
            }}
          >
            {cancelText}
          </button>
          <button
            ref={confirmButtonRef}
            onClick={onConfirm}
            aria-label={confirmText}
            style={{
              padding: "8px 20px",
              borderRadius: 8,
              border: "none",
              background: "#6366f1",
              color: "white",
              cursor: "pointer"
            }}
          >
            {confirmText}
          </button>
        </div>
      </div>
    </div>
  );
};
