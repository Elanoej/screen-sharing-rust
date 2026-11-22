import ReactDOM from "react-dom/client";
import "./index.css";
import App from "./App";

// Sem React.StrictMode de propósito: em dev ele monta/desmonta os efeitos duas
// vezes, o que abriria o seletor de tela do portal (Wayland) duas vezes e
// iniciaria uma captura descartada.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <App />,
);
