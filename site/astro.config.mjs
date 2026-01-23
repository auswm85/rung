import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// https://astro.build/config
export default defineConfig({
  site: "https://rungstack.com",
  integrations: [
    starlight({
      title: "rung",
      description:
        "Stacked PRs made simple. A CLI tool for managing stacked pull requests on GitHub.",
      logo: {
        src: "./src/assets/logo.png",
        replacesTitle: false,
      },
      social: {
        github: "https://github.com/auswm85/rung",
      },
      editLink: {
        baseUrl: "https://github.com/auswm85/rung/edit/main/site/",
      },
      customCss: ["./src/styles/custom.css"],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Installation", slug: "getting-started/installation" },
            { label: "Quick Start", slug: "getting-started/quickstart" },
          ],
        },
        {
          label: "Commands",
          items: [
            { label: "Overview", slug: "commands" },
            { label: "init", slug: "commands/init" },
            { label: "create", slug: "commands/create" },
            { label: "status", slug: "commands/status" },
            { label: "sync", slug: "commands/sync" },
            { label: "submit", slug: "commands/submit" },
            { label: "merge", slug: "commands/merge" },
            { label: "restack", slug: "commands/restack" },
            {
              label: "navigation (nxt, prv, move)",
              slug: "commands/navigation",
            },
            { label: "log", slug: "commands/log" },
            { label: "absorb", slug: "commands/absorb" },
            { label: "undo", slug: "commands/undo" },
            { label: "doctor", slug: "commands/doctor" },
            { label: "update", slug: "commands/update" },
            { label: "completions", slug: "commands/completions" },
          ],
        },
        {
          label: "Guides",
          items: [
            { label: "What are Stacked PRs?", slug: "guides/stacked-prs" },
            { label: "Basic Workflow", slug: "guides/basic-workflow" },
            {
              label: "Conflict Resolution",
              slug: "guides/conflict-resolution",
            },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "Configuration", slug: "reference/configuration" },
            { label: "Troubleshooting", slug: "reference/troubleshooting" },
            { label: "FAQ", slug: "reference/faq" },
          ],
        },
      ],
    }),
  ],
});
