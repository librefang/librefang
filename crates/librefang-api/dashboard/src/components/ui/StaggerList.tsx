import { Children, isValidElement, type ReactNode } from "react";
import { AnimatePresence, motion, type HTMLMotionProps } from "motion/react";
import { staggerContainer, staggerItem } from "../../lib/motion";

interface StaggerListProps extends Omit<HTMLMotionProps<"div">, "variants" | "initial" | "animate" | "children"> {
  children: ReactNode;
  /** Skip wrapping each child in a motion.div — useful when callers
   *  already provide motion children with their own variants. */
  manualItems?: boolean;
  /** Disable enter/exit animations for individual items. When the
   *  list contents are stable (hardcoded cards, not driven by data),
   *  the AnimatePresence overhead is unnecessary. Defaults to false. */
  staticItems?: boolean;
}

/// Drop-in replacement for the legacy `.stagger-children` className.
///
/// Wraps each direct child in a `motion.div` that inherits the
/// `staggerItem` variant from the container, producing the same 40ms
/// cascade the CSS implementation produced.
///
/// When children come from a `.map()` over data, items animate in/out
/// on add/remove via `<AnimatePresence>` and `layout` reflows the grid
/// smoothly. Children must have stable keys (data IDs, not array index)
/// for the exit animation to play.
///
/// Usage:
///   <StaggerList className="grid grid-cols-3 gap-4">
///     {items.map(item => <Card key={item.id}>…</Card>)}
///   </StaggerList>
export function StaggerList({ children, manualItems, staticItems, ...rest }: StaggerListProps) {
  if (manualItems) {
    return (
      <motion.div
        variants={staggerContainer}
        initial="initial"
        animate="animate"
        {...rest}
      >
        {children}
      </motion.div>
    );
  }

  const wrapped = Children.map(children, (child, idx) => {
    if (!isValidElement(child)) return child;
    const key = (child as { key?: string | number | null }).key ?? idx;
    return (
      <motion.div
        key={key}
        variants={staggerItem}
        layout={!staticItems}
        {...(staticItems ? {} : { exit: "exit" as const })}
      >
        {child}
      </motion.div>
    );
  });

  return (
    <motion.div
      variants={staggerContainer}
      initial="initial"
      animate="animate"
      {...rest}
    >
      {staticItems ? wrapped : <AnimatePresence mode="popLayout">{wrapped}</AnimatePresence>}
    </motion.div>
  );
}
