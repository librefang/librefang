import { Children, isValidElement, type ReactNode } from "react";
import { motion, type HTMLMotionProps } from "motion/react";
import { staggerContainer, staggerItem } from "../../lib/motion";

interface StaggerListProps extends Omit<HTMLMotionProps<"div">, "variants" | "initial" | "animate" | "children"> {
  children: ReactNode;
}

/// Drop-in replacement for the legacy `.stagger-children` className.
///
/// Wraps each direct child in a `motion.div` that inherits the
/// `staggerItem` variant from the container, producing the same 40ms
/// cascade the CSS implementation produced.
///
/// Behaviour: enter-only animation. We deliberately do NOT use `layout`
/// / `<AnimatePresence>` / `popLayout` here. Those add exit animation
/// and neighbour reflow, but motion's `layout` toggles
/// `pointer-events: none` on the wrapped element while a layout
/// animation is running — which silently breaks click handling on
/// click-to-open cards (Hands, Plugins, etc) any time the surrounding
/// list re-measures (refetch, font load, viewport resize). Match the
/// old CSS exactly: items fade in, deletions just disappear.
///
/// Usage:
///   <StaggerList className="grid grid-cols-3 gap-4">
///     {items.map(item => <Card key={item.id}>…</Card>)}
///   </StaggerList>
export function StaggerList({ children, ...rest }: StaggerListProps) {
  return (
    <motion.div
      variants={staggerContainer}
      initial="initial"
      animate="animate"
      {...rest}
    >
      {Children.map(children, (child, idx) => {
        if (!isValidElement(child)) return child;
        const key = (child as { key?: string | number | null }).key ?? idx;
        return (
          <motion.div key={key} variants={staggerItem}>
            {child}
          </motion.div>
        );
      })}
    </motion.div>
  );
}
