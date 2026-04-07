"use client";

import { useEffect, useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useKolTasks } from "../hooks/use-kol-tasks";
import {
  AliasTypeLabel,
  WriteBackStatusLabel,
  type AliasType,
  type WriteBackStatus,
  type TaskQueryRequest,
} from "../types";

export function KolTaskPanel() {
  const { taskGrid, loading, fetchTaskGrid } = useKolTasks();
  const [query, setQuery] = useState<TaskQueryRequest>({
    page: 1,
    page_size: 20,
    date_range: "day",
  });

  useEffect(() => {
    fetchTaskGrid(query);
  }, [fetchTaskGrid, query]);

  const handleDateChange = (range: string) => {
    setQuery((prev) => ({ ...prev, date_range: range as "day" | "week" | "month", page: 1 }));
  };

  const handlePageChange = (page: number) => {
    setQuery((prev) => ({ ...prev, page }));
  };

  const totalPages = taskGrid ? Math.ceil(taskGrid.total / taskGrid.page_size) : 0;

  return (
    <div className="space-y-4">
      {/* Filters */}
      <div className="flex items-center gap-4">
        <Select value={query.date_range || "day"} onValueChange={handleDateChange}>
          <SelectTrigger className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="day">今日</SelectItem>
            <SelectItem value="week">本周</SelectItem>
            <SelectItem value="month">本月</SelectItem>
          </SelectContent>
        </Select>

        <div className="text-sm text-muted-foreground">
          共 {taskGrid?.total ?? 0} 条
        </div>
      </div>

      {/* Task Table */}
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-16">ID</TableHead>
                <TableHead>关键词</TableHead>
                <TableHead className="w-20">平台</TableHead>
                <TableHead className="w-24">回填状态</TableHead>
                <TableHead>分享链接</TableHead>
                <TableHead className="w-16">KOL</TableHead>
                <TableHead>创建时间</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {!taskGrid || taskGrid.items.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center text-muted-foreground">
                    暂无数据
                  </TableCell>
                </TableRow>
              ) : (
                taskGrid.items.map((task) => (
                  <TableRow key={task.id}>
                    <TableCell className="font-mono text-xs">{task.id}</TableCell>
                    <TableCell className="font-medium">{task.alias_name}</TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {AliasTypeLabel[task.platform as AliasType] ?? task.platform}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          task.write_back_status === 1
                            ? "default"
                            : task.write_back_status === 2
                              ? "secondary"
                              : "outline"
                        }
                      >
                        {WriteBackStatusLabel[task.write_back_status as WriteBackStatus] ??
                          task.write_back_status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-xs max-w-48 truncate">
                      {task.share_url || "-"}
                    </TableCell>
                    <TableCell className="text-xs">{task.kol_id}</TableCell>
                    <TableCell className="text-xs">
                      {new Date(task.created_at).toLocaleString()}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Pagination */}
      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={query.page === 1}
            onClick={() => handlePageChange((query.page || 1) - 1)}
          >
            上一页
          </Button>
          <span className="text-sm text-muted-foreground">
            {query.page} / {totalPages}
          </span>
          <Button
            size="sm"
            variant="outline"
            disabled={query.page === totalPages}
            onClick={() => handlePageChange((query.page || 1) + 1)}
          >
            下一页
          </Button>
        </div>
      )}
    </div>
  );
}
