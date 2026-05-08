"use client";

import { useCallback, useEffect, useState } from "react";
import { kolApi } from "../api/client";
import type { CreateUserRequest, UpdateUserRequest, User } from "../types";

export function useKolAdminUsers() {
  const [users, setUsers] = useState<User[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setUsers(await kolApi.listUsers());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const create = useCallback(async (req: CreateUserRequest) => {
    const created = await kolApi.createUser(req);
    setUsers((prev) => [...prev, created]);
    return created;
  }, []);

  const update = useCallback(async (id: number, req: UpdateUserRequest) => {
    const updated = await kolApi.updateUser(id, req);
    setUsers((prev) => prev.map((u) => (u.id === id ? updated : u)));
    return updated;
  }, []);

  const remove = useCallback(async (id: number) => {
    await kolApi.deleteUser(id);
    setUsers((prev) => prev.filter((u) => u.id !== id));
  }, []);

  return { users, loading, error, refresh, create, update, remove };
}
