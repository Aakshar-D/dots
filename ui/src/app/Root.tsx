import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { FatalPage } from "@/app/FatalPage";
import { Layout } from "@/app/Layout";
import { errorMessage } from "@/lib/notify";
import { keys } from "@/lib/query";

/// The app, or the reason it could not start.
export function Root() {
  const startup = useQuery({ queryKey: keys.startupError, queryFn: commands.startupError });
  if (startup.isPending) return null;
  if (startup.isError) return <FatalPage message={errorMessage(startup.error)} logDir={null} />;
  if (startup.data) {
    return <FatalPage message={startup.data.message} logDir={startup.data.log_dir} />;
  }
  return <Layout />;
}
