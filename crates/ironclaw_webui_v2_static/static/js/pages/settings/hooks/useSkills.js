import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchSkillContent,
  fetchSkills,
  installSkill as installSkillRequest,
  removeSkill as removeSkillRequest,
  updateSkill as updateSkillRequest,
} from "../lib/settings-api.js";

export function useSkills() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["skills"],
    queryFn: fetchSkills,
  });

  const installMutation = useMutation({
    mutationFn: installSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const removeMutation = useMutation({
    mutationFn: removeSkillRequest,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ name, content }) => updateSkillRequest(name, { content }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
  });

  const skills = query.data?.skills || [];
  // No v2 skills endpoint exists yet: `fetchSkills` may return `{ todo: true }`
  // and the install/remove writes are `{ success: false }` stubs. `status:'todo'`
  // lets the tab gate the import form behind a real backend so users never submit
  // an install that silently no-ops ("No fake readiness"). The skill-edit feature
  // remains backed by the real `fetchSkillContent`/`updateSkill` endpoints.
  const status = query.data?.todo ? 'todo' : 'ready';

  return {
    skills,
    query,
    status,
    fetchSkillContent,
    installSkill: installMutation.mutateAsync,
    removeSkill: removeMutation.mutateAsync,
    updateSkill: updateMutation.mutateAsync,
    isInstalling: installMutation.isPending,
    isRemoving: removeMutation.isPending,
    isUpdating: updateMutation.isPending,
  };
}
