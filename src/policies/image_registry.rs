use kube::core::admission::AdmissionRequest;
use kube::core::DynamicObject;

use crate::config::AllowedRegistriesPolicy;

use super::{container_name, get_containers, get_pod_spec, PolicyOutput};

pub fn evaluate(
    config: &AllowedRegistriesPolicy,
    request: &AdmissionRequest<DynamicObject>,
) -> PolicyOutput {
    let object = match &request.object {
        Some(obj) => obj,
        None => return PolicyOutput::allowed(),
    };

    let kind = &request.kind.kind;
    let pod_spec = match get_pod_spec(&object.data, kind) {
        Some(spec) => spec,
        None => return PolicyOutput::allowed(),
    };

    let containers = get_containers(pod_spec);
    let mut violations = Vec::new();

    for (_, container) in &containers {
        let name = container_name(container);
        let image = match container.get("image").and_then(|v| v.as_str()) {
            Some(img) => img,
            None => {
                violations.push(format!(
                    "container '{name}' has no image specified"
                ));
                continue;
            }
        };

        let ImageRef { registry, tag, has_digest } = parse_image_ref(image);

        let registry_allowed = config
            .registries
            .iter()
            .any(|allowed| registry_matches(&registry, allowed));

        if !registry_allowed {
            violations.push(format!(
                "container '{name}' image '{image}' uses registry '{registry}' \
                 which is not in the allowed list [{}]",
                config.registries.join(", ")
            ));
        }

        if !config.allow_latest_tag {
            let is_latest = tag == "latest" || (tag.is_empty() && !has_digest);
            if is_latest {
                let tag_display = if tag.is_empty() {
                    "<none> (defaults to latest)"
                } else {
                    "latest"
                };
                violations.push(format!(
                    "container '{name}' image '{image}' uses tag '{tag_display}'"
                ));
            }
        }
    }

    PolicyOutput {
        violations,
        patches: Vec::new(),
    }
}

fn registry_matches(registry: &str, allowed: &str) -> bool {
    if registry == allowed {
        return true;
    }
    if registry.starts_with(allowed) {
        let next_byte = registry.as_bytes().get(allowed.len());
        return matches!(next_byte, Some(b'/'));
    }
    false
}

struct ImageRef {
    registry: String,
    tag: String,
    has_digest: bool,
}

fn parse_image_ref(image: &str) -> ImageRef {
    let has_digest = image.contains('@');

    let image_no_digest = if let Some(pos) = image.find('@') {
        &image[..pos]
    } else {
        image
    };

    let (name_part, tag) = if let Some(last_slash) = image_no_digest.rfind('/') {
        if let Some(colon_offset) = image_no_digest[last_slash..].find(':') {
            let colon_pos = last_slash + colon_offset;
            (
                &image_no_digest[..colon_pos],
                &image_no_digest[colon_pos + 1..],
            )
        } else {
            (image_no_digest, "")
        }
    } else if let Some(colon_pos) = image_no_digest.find(':') {
        (
            &image_no_digest[..colon_pos],
            &image_no_digest[colon_pos + 1..],
        )
    } else {
        (image_no_digest, "")
    };

    let registry = extract_registry(name_part);

    ImageRef { registry, tag: tag.to_string(), has_digest }
}

fn extract_registry(name_part: &str) -> String {
    if let Some(slash_pos) = name_part.find('/') {
        let first = &name_part[..slash_pos];
        let has_explicit_registry =
            first.contains('.') || first.contains(':') || first == "localhost";

        if has_explicit_registry {
            match name_part.rfind('/') {
                Some(pos) => name_part[..pos].to_string(),
                None => name_part.to_string(),
            }
        } else {
            format!("docker.io/{first}")
        }
    } else {
        "docker.io/library".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_image_ref() {
        let ImageRef { registry, tag, has_digest } = parse_image_ref("nginx");
        assert_eq!(registry, "docker.io/library");
        assert_eq!(tag, "");
        assert!(!has_digest);

        let ImageRef { registry, tag, has_digest } = parse_image_ref("nginx:latest");
        assert_eq!(registry, "docker.io/library");
        assert_eq!(tag, "latest");
        assert!(!has_digest);

        let ImageRef { registry, tag, has_digest } = parse_image_ref("nginx:1.25");
        assert_eq!(registry, "docker.io/library");
        assert_eq!(tag, "1.25");
        assert!(!has_digest);

        let ImageRef { registry, tag, has_digest } = parse_image_ref("myuser/myapp:v2");
        assert_eq!(registry, "docker.io/myuser");
        assert_eq!(tag, "v2");
        assert!(!has_digest);

        let ImageRef { registry, tag, has_digest } = parse_image_ref("gcr.io/my-project/my-image:v1.0");
        assert_eq!(registry, "gcr.io/my-project");
        assert_eq!(tag, "v1.0");
        assert!(!has_digest);

        let ImageRef { registry, tag, has_digest } =
            parse_image_ref("gcr.io/my-project/my-image@sha256:abcdef1234567890");
        assert_eq!(registry, "gcr.io/my-project");
        assert_eq!(tag, "");
        assert!(has_digest);

        let ImageRef { registry, tag, has_digest } = parse_image_ref("localhost:5000/myimage:v1");
        assert_eq!(registry, "localhost:5000");
        assert_eq!(tag, "v1");
        assert!(!has_digest);
    }

    #[test]
    fn test_registry_matches() {
        assert!(registry_matches("gcr.io/project", "gcr.io"));
        assert!(registry_matches("gcr.io", "gcr.io"));
        assert!(!registry_matches("gcr.io.evil.com", "gcr.io"));
        assert!(registry_matches("docker.io/library", "docker.io"));
        assert!(!registry_matches("docker.io.fake", "docker.io"));
    }
}
