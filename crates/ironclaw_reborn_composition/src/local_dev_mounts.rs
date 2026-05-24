use std::path::Path;

use ironclaw_host_api::{
    HostApiError, MountAlias, MountGrant, MountPermissions, MountView, VirtualPath,
};

const WORKSPACE_ALIAS: &str = "/workspace";
const WORKSPACE_TARGET: &str = "/projects/workspace";
const HOST_ALIAS: &str = "/host";
const HOST_TARGET: &str = "/projects/host";

pub(crate) fn workspace_mount_view(
    permissions: MountPermissions,
    host_home_alias: Option<&Path>,
) -> Result<MountView, HostApiError> {
    let mut mounts = vec![grant(
        WORKSPACE_ALIAS,
        WORKSPACE_TARGET,
        permissions.clone(),
    )?];
    if let Some(host_home_alias) = host_home_alias {
        mounts.push(grant(HOST_ALIAS, HOST_TARGET, permissions.clone())?);
        if let Some(host_home_alias) = host_home_alias.to_str() {
            if let Ok(raw_host_home_alias) = MountAlias::new(host_home_alias) {
                mounts.push(MountGrant::new(
                    raw_host_home_alias,
                    VirtualPath::new(HOST_TARGET)?,
                    permissions,
                ));
            }
        }
    }
    MountView::new(mounts)
}

pub(crate) fn skill_context_mount_view() -> Result<MountView, HostApiError> {
    MountView::new(vec![
        grant("/skills", "/projects/skills", MountPermissions::read_only())?,
        grant(
            "/tenant-shared/skills",
            "/projects/tenant-shared/skills",
            MountPermissions::read_only(),
        )?,
        grant(
            "/system/skills",
            "/projects/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
}

pub(crate) fn skill_management_mount_view() -> Result<MountView, HostApiError> {
    MountView::new(vec![
        grant(
            "/skills",
            "/projects/skills",
            MountPermissions::read_write_list_delete(),
        )?,
        grant(
            "/system/skills",
            "/projects/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
}

fn grant(
    alias: &str,
    target: &str,
    permissions: MountPermissions,
) -> Result<MountGrant, HostApiError> {
    Ok(MountGrant::new(
        MountAlias::new(alias)?,
        VirtualPath::new(target)?,
        permissions,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_mount_keeps_host_alias_when_raw_alias_is_not_mount_shaped() {
        let mounts = workspace_mount_view(
            MountPermissions::read_write(),
            Some(Path::new(r"C:\Users\alice")),
        )
        .expect("mount view builds");

        assert!(
            mounts
                .mounts
                .iter()
                .any(|mount| mount.alias.as_str() == "/host")
        );
        assert!(
            mounts
                .mounts
                .iter()
                .all(|mount| mount.alias.as_str() != r"C:\Users\alice")
        );
    }
}
