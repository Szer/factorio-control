export type Page = "overview" | "worlds" | "players" | "settings" | "console";
export function readPage(): Page {
  switch (window.location.hash) {
    case "#worlds":
      return "worlds";
    case "#players":
      return "players";
    case "#settings":
      return "settings";
    case "#console":
      return "console";
    default:
      return "overview";
  }
}
