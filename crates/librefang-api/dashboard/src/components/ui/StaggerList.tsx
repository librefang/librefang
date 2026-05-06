import { Children, isValidElement, type ReactNode } from "react";
import { motion, type HTMLMotionProps } from "motion/react";
import { staggerContainer, staggerItem } from "../../lib/motion";

const motionComponentCache = new Map<string, ReturnType<typeof motion.create>>();

function getMotionComponent(tag: string) {
  let component = motionComponentCache.get(tag);
  if (!component) {
    component = motion.create(tag);
    motionComponentCache.set(tag, component);
  }
  return component;
}

type StaggerItemElement = "div" | "li" | "span" | "article" | "section";
type ContainerElement = "div" | "ul" | "ol" | "section" | "article";

const CONTAINER_DEFAULTS: Record<StaggerItemElement, ContainerElement> = {
  li: "ul",
  div: "div",
  span: "div",
  article: "div",
  section: "div",
};

interface StaggerListProps extends Omit<HTMLMotionProps<"div">, "variants" | "initial" | "animate" | "children"> {
  children: ReactNode;
  as?: StaggerItemElement;
  containerAs?: ContainerElement;
}

/// Drop-in replacement for the legacy `.stagger-children` className.
///
/// Wraps each direct child in a motion element that inherits the
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
///
/// List semantics — set `as` to match the child element type;
/// container auto-derives (e.g. as="li" → container is "ul"):
///   <StaggerList as="li">
///     {items.map(item => <li key={item.id}>…</li>)}
///   </StaggerList>
///
/// Override the container explicitly with `containerAs`:
///   <StaggerList as="li" containerAs="ol">
export function StaggerList({ children, as: elementType = "div", containerAs, ...rest }: StaggerListProps) {
  const MotionItem = getMotionComponent(elementType);
  const containerTag = containerAs ?? CONTAINER_DEFAULTS[elementType];
  const MotionContainer = getMotionComponent(containerTag);

  return (
    <MotionContainer
      variants={staggerContainer}
      initial="initial"
      animate="animate"
      {...rest}
    >
      {Children.map(children, (child, idx) => {
        if (!isValidElement(child)) return child;
        const key = child.key ?? idx;
        return (
          <MotionItem key={key} variants={staggerItem}>
            {child}
          </MotionItem>
        );
      })}
    </MotionContainer>
  );
}
