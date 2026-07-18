export type AgentChipDisplay = {
  kind: "image" | "text";
  label: string;
  url: string;
};

export function agentChipDisplay(
  agentName: string,
  agentAvatarUrl: string,
  imageFailed: boolean,
): AgentChipDisplay {
  const label = agentInitial(agentName);
  const url = agentAvatarUrl.trim();

  if (url && !imageFailed) {
    return { kind: "image", label, url };
  }

  return { kind: "text", label, url: "" };
}

export function agentInitial(agentName: string): string {
  return agentName.trim().slice(0, 2).toUpperCase() || "??";
}
