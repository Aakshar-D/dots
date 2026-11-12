import { useQuery } from "@tanstack/react-query";
import { Check } from "lucide-react";

import { commands } from "@/api/commands";
import { Role } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { roleLabel } from "@/lib/access";
import { keys } from "@/lib/query";

/// "What each role can do" (guardrails spec section 5): the core's permission table as it
/// checks it, with each condition under its row. The UI only shows it, never decides a role.
export function RoleMatrix() {
  const matrix = useQuery({ queryKey: keys.roleMatrix, queryFn: commands.roleMatrix });
  return (
    <details className="flex flex-col gap-2 rounded-md border p-2">
      <summary className="cursor-pointer text-sm font-medium">What each role can do</summary>
      {matrix.isError && <ErrorNote error={matrix.error} />}
      {matrix.data && (
        <Table aria-label="What each role can do">
          <TableHeader>
            <TableRow>
              <TableHead scope="col">Can</TableHead>
              {Role.options.map((role) => (
                <TableHead key={role} scope="col" className="text-center">
                  {roleLabel(role)}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {matrix.data.map((row) => (
              <TableRow key={row.permission}>
                <TableHead scope="row" className="h-auto py-2 font-normal whitespace-normal">
                  <span className="block first-letter:uppercase">{row.action}</span>
                  {row.note && (
                    <span className="block text-xs text-muted-foreground">{row.note}</span>
                  )}
                </TableHead>
                {Role.options.map((role) => (
                  <TableCell key={role} className="text-center">
                    {row.roles.includes(role) ? (
                      <>
                        <Check aria-hidden className="inline size-4" />
                        <span className="sr-only">Yes</span>
                      </>
                    ) : (
                      <>
                        <span aria-hidden className="text-muted-foreground">
                          –
                        </span>
                        <span className="sr-only">No</span>
                      </>
                    )}
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </details>
  );
}
