export type Page = "overview" | "saves" | "mods" | "settings" | "console";
export function readPage(): Page {
  switch (window.location.hash) {
    case "#saves":
      return "saves";
    case "#mods":
      return "mods";
    case "#settings":
      return "settings";
    case "#console":
      return "console";
    default:
      return "overview";
  }
}
