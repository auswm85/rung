import { defineCollection, z } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({
      extend: z.object({
        // Version when this feature was introduced
        // Use "unreleased" for features not yet released
        since: z.string().optional(),
      }),
    }),
  }),
};
