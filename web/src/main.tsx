import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import AdminPanel from "./components/AdminPanel";
import "./index.css";

const isAdminRoute = () => window.location.hash.replace(/^#\/?/, "") === "admin";

/** 极简路由：`#admin` 进入管理后台（专供管理员），其余进入市场。 */
function Root() {
  const [admin, setAdmin] = useState(isAdminRoute());
  useEffect(() => {
    const onHash = () => setAdmin(isAdminRoute());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);
  return admin ? <AdminPanel /> : <App />;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
