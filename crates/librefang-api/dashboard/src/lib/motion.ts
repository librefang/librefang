import type { Variants } from "motion/react";

// Apple-style easing curves, mirror the CSS variables in index.css so
// motion-driven transitions match the rest of the design system.
export const APPLE_EASE: [number, number, number, number] = [0.25, 0.1, 0.25, 1];
export const APPLE_SPRING: [number, number, number, number] = [0.22, 1, 0.36, 1];
export const APPLE_BOUNCE: [number, number, number, number] = [0.34, 1.56, 0.64, 1];

// Modal / dialog entrance: scale up + deblur. Replaces .animate-fade-in-scale.
export const fadeInScale: Variants = {
  initial: { opacity: 0, scale: 0.92, filter: "blur(8px)" },
  animate: {
    opacity: 1,
    scale: 1,
    filter: "blur(0px)",
    transition: { duration: 0.5, ease: APPLE_BOUNCE },
  },
  exit: {
    opacity: 0,
    scale: 0.96,
    filter: "blur(6px)",
    transition: { duration: 0.22, ease: APPLE_EASE },
  },
};

// Page / hero entrance: fade + 16px rise + deblur. Replaces .animate-fade-in-up.
export const fadeInUp: Variants = {
  initial: { opacity: 0, y: 16, filter: "blur(4px)" },
  animate: {
    opacity: 1,
    y: 0,
    filter: "blur(0px)",
    transition: { duration: 0.6, ease: APPLE_SPRING },
  },
  exit: {
    opacity: 0,
    y: 8,
    transition: { duration: 0.22, ease: APPLE_EASE },
  },
};

// Lightweight chat-message entrance: 6px rise, no blur. Replaces .animate-message-in.
export const messageIn: Variants = {
  initial: { opacity: 0, y: 6 },
  animate: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.22, ease: APPLE_EASE },
  },
};

// Right-docked drawer/panel entrance. Replaces .animate-slide-in-right.
export const slideInRight: Variants = {
  initial: { x: "100%", opacity: 0.6 },
  animate: {
    x: 0,
    opacity: 1,
    transition: { duration: 0.28, ease: APPLE_EASE },
  },
  exit: {
    x: "100%",
    opacity: 0.6,
    transition: { duration: 0.28, ease: APPLE_EASE },
  },
};

// Stagger container — 40ms cascade, matches the old CSS .stagger-children
// (10 children × 40ms = 360ms). Use with `<StaggerList>` or attach manually.
export const staggerContainer: Variants = {
  initial: {},
  animate: {
    transition: { staggerChildren: 0.04, delayChildren: 0 },
  },
};

// Single staggered child — same shape as fadeInUp but slightly faster (0.5s).
// Includes an `exit` variant so list removal inside <AnimatePresence>
// plays a smooth fade-and-collapse instead of disappearing instantly.
export const staggerItem: Variants = {
  initial: { opacity: 0, y: 16, filter: "blur(4px)" },
  animate: {
    opacity: 1,
    y: 0,
    filter: "blur(0px)",
    transition: { duration: 0.5, ease: APPLE_SPRING },
  },
  exit: {
    opacity: 0,
    y: -8,
    scale: 0.96,
    transition: { duration: 0.18, ease: APPLE_EASE },
  },
};

// Route-level entrance — a quick fade + 8px rise. Keep it short (180ms) so
// page transitions feel snappy; longer durations make navigation feel laggy.
export const pageTransition: Variants = {
  initial: { opacity: 0, y: 8 },
  animate: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.18, ease: APPLE_EASE },
  },
  exit: {
    opacity: 0,
    y: -8,
    transition: { duration: 0.12, ease: APPLE_EASE },
  },
};

// Tab content swap — like pageTransition but horizontal, smaller travel.
export const tabContent: Variants = {
  initial: { opacity: 0, x: 12 },
  animate: {
    opacity: 1,
    x: 0,
    transition: { duration: 0.18, ease: APPLE_EASE },
  },
  exit: {
    opacity: 0,
    x: -12,
    transition: { duration: 0.12, ease: APPLE_EASE },
  },
};

// Toast slide-in from the right edge — replaces tailwindcss-animate's
// slide-in-from-right-5. Includes a tidy exit so dismissed toasts don't
// just vanish.
export const toastSlide: Variants = {
  initial: { opacity: 0, x: 32, scale: 0.95 },
  animate: {
    opacity: 1,
    x: 0,
    scale: 1,
    transition: { duration: 0.24, ease: APPLE_EASE },
  },
  exit: {
    opacity: 0,
    x: 32,
    scale: 0.95,
    transition: { duration: 0.18, ease: APPLE_EASE },
  },
};
