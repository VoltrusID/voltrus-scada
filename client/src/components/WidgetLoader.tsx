import React, { useState, useEffect, useRef } from "react";

interface WidgetLoaderProps {
  widgetId: string;
  data: Record<string, any>;
  config: Record<string, any>;
  width: number;
  height: number;
  theme: "light" | "dark";
  locale: string;
}

const WidgetErrorFallback: React.FC<{ widgetId: string }> = ({ widgetId }) => (
  <div style={{
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    height: "100%",
    background: "#fef2f2",
    border: "1px solid #fecaca",
    borderRadius: "8px",
    color: "#dc2626",
    fontSize: "13px",
  }}>
    Widget Error: {widgetId}
  </div>
);

export const WidgetLoader: React.FC<WidgetLoaderProps> = ({
  widgetId,
  data,
  config,
  width,
  height,
  theme,
  locale,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    if (error || !containerRef.current) return;

    const loadWidget = async () => {
      try {
        // Load widget CSS if present
        const existingLink = containerRef.current!.querySelector(
          `link[data-widget-css="${widgetId}"]`
        );
        if (!existingLink) {
          const link = document.createElement("link");
          link.rel = "stylesheet";
          link.href = `/api/v1/widgets/${widgetId}/asset/widget.css`;
          link.setAttribute("data-widget-css", widgetId);
          containerRef.current!.appendChild(link);
        }

        // Create shadow DOM for isolation
        const shadow = containerRef.current!.shadowRoot ?? containerRef.current!.attachShadow({ mode: "open" });

        // Load widget JS dynamically
        const script = document.createElement("script");
        script.src = `/api/v1/widgets/${widgetId}/asset/widget.js`;
        script.async = true;
        script.onload = () => {
          if (shadow && (window as any)[`VoltrusWidget_${widgetId}`]) {
            const WidgetComponent = (window as any)[`VoltrusWidget_${widgetId}`];
            const mountPoint = document.createElement("div");
            shadow.appendChild(mountPoint);

            WidgetComponent(mountPoint, {
              data,
              config,
              width,
              height,
              theme,
              locale,
            });
          }
        };
        script.onerror = () => setError(true);
        containerRef.current!.appendChild(script);
      } catch (e) {
        console.error(`Widget ${widgetId} load error:`, e);
        setError(true);
      }
    };

    loadWidget();
  }, [widgetId, data, config, width, height, theme, locale, error]);

  if (error) {
    return <WidgetErrorFallback widgetId={widgetId} />;
  }

  return (
    <div
      ref={containerRef}
      style={{ width: "100%", height: "100%" }}
    />
  );
};
