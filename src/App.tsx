import { getCurrentWindow } from "@tauri-apps/api/window";
import { MainView } from "./views/MainView";
import { QuickPanel } from "./views/QuickPanel";

export default function App() {
  return getCurrentWindow().label === "quick" ? <QuickPanel /> : <MainView />;
}
