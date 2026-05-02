import type { AgentItem, HandInstanceItem } from "../api";

export type HandGroupAgent = AgentItem & {
  role: string;
  isCoordinator: boolean;
};

export interface HandGroup {
  hand_id: string;
  hand_name: string;
  hand_icon?: string;
  agents: HandGroupAgent[];
}

export interface GroupedPicker {
  standalone: AgentItem[];
  handGroups: HandGroup[];
}

// A hand instance is "usable" for the chat picker only if it's currently
// Active, has at least one role→agent mapping, and carries the hand_id /
// hand_name metadata we need to render its group header. Anything else is
// dropped so paused / partially-spawned / stripped instances don't surface
// orphaned agents in the list. (See Q1/Q6 in the design spec.)
export function isUsableHandInstance(h: HandInstanceItem): boolean {
  return (
    (h.status ?? "") === "Active" &&
    !!h.agent_ids &&
    Object.keys(h.agent_ids).length > 0 &&
    h.hand_id !== undefined &&
    h.hand_name !== undefined
  );
}

export function groupedPicker(
  agents: AgentItem[],
  handInstances: HandInstanceItem[] | undefined,
  showHandAgents: boolean,
): GroupedPicker {
  if (!showHandAgents) {
    return {
      standalone: agents.filter((a) => !a.is_hand),
      handGroups: [],
    };
  }

  // Distinguish "still loading" from "no active hands". When `handInstances`
  // is undefined the hands query hasn't resolved yet; dropping is_hand agents
  // here would let URL-pinned hand agents disappear from the picker during the
  // bootstrap window (issue #4296 Bug A). Surface them as standalone until
  // the second query resolves; the next render with real data re-groups them.
  if (handInstances === undefined) {
    return { standalone: agents.slice(), handGroups: [] };
  }

  const activeHands = handInstances.filter(isUsableHandInstance);

  // Build agent_id → { hand metadata, role, isCoordinator } lookup.
  type Membership = {
    hand_id: string;
    hand_name: string;
    hand_icon?: string;
    role: string;
    isCoordinator: boolean;
  };
  const lookup = new Map<string, Membership>();
  for (const h of activeHands) {
    const ids = h.agent_ids ?? {};
    for (const [role, agentId] of Object.entries(ids)) {
      lookup.set(agentId, {
        hand_id: h.hand_id!,
        hand_name: h.hand_name!,
        hand_icon: h.hand_icon,
        role,
        isCoordinator: h.coordinator_role === role,
      });
    }
  }

  // Partition agents into standalone vs grouped.
  const standalone: AgentItem[] = [];
  const groupsByHandId = new Map<string, HandGroup>();
  for (const agent of agents) {
    const membership = lookup.get(agent.id);
    if (!membership) {
      if (!agent.is_hand) {
        standalone.push(agent);
      }
      // is_hand agents whose hand is not in the active lookup are dropped
      // entirely (Q1 / Paused-hand test case in chatPicker.test.ts).
      continue;
    }
    let group = groupsByHandId.get(membership.hand_id);
    if (!group) {
      group = {
        hand_id: membership.hand_id,
        hand_name: membership.hand_name,
        hand_icon: membership.hand_icon,
        agents: [],
      };
      groupsByHandId.set(membership.hand_id, group);
    }
    group.agents.push({
      ...agent,
      role: membership.role,
      isCoordinator: membership.isCoordinator,
    });
  }

  // Sort within each group: coordinator first, then alphabetical by role.
  for (const group of groupsByHandId.values()) {
    group.agents.sort((a, b) => {
      if (a.isCoordinator && !b.isCoordinator) return -1;
      if (!a.isCoordinator && b.isCoordinator) return 1;
      return a.role.localeCompare(b.role);
    });
  }

  // Sort groups alphabetically by hand_name.
  const handGroups = Array.from(groupsByHandId.values()).sort((a, b) =>
    a.hand_name.localeCompare(b.hand_name),
  );

  return { standalone, handGroups };
}
