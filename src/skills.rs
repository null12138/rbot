use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub system_prompt: Option<String>,
    pub allowed_tools: Vec<String>,
    pub allowed_commands: Vec<String>,
}

#[derive(Clone)]
pub struct SkillManager {
    skills: Arc<RwLock<HashMap<String, Skill>>>,
    active: Arc<RwLock<HashMap<i64, String>>>,
}

impl SkillManager {
    pub fn load(dir: &str) -> anyhow::Result<Self> {
        let mut map = HashMap::new();
        let path = PathBuf::from(dir);
        if path.exists() {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                if !entry.path().is_file() {
                    continue;
                }
                if entry.path().extension().and_then(|s| s.to_str()) != Some("toml") {
                    continue;
                }
                let text = fs::read_to_string(entry.path())?;
                let skill: Skill = toml::from_str(&text)?;
                map.insert(skill.name.clone(), skill);
            }
        }
        Ok(Self {
            skills: Arc::new(RwLock::new(map)),
            active: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn list(&self) -> Vec<Skill> {
        self.skills.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<Skill> {
        self.skills.read().unwrap().get(name).cloned()
    }

    pub fn activate(&self, chat_id: i64, name: &str) -> anyhow::Result<()> {
        if self.skills.read().unwrap().contains_key(name) {
            self.active.write().unwrap().insert(chat_id, name.to_string());
            Ok(())
        } else {
            anyhow::bail!("skill not found")
        }
    }

    pub fn deactivate(&self, chat_id: i64) {
        self.active.write().unwrap().remove(&chat_id);
    }

    pub fn active_skill(&self, chat_id: i64) -> Option<Skill> {
        let name = self.active.read().unwrap().get(&chat_id).cloned();
        name.and_then(|n| self.get(&n))
    }

    pub fn maybe_trigger(&self, chat_id: i64, text: &str) -> Option<Skill> {
        let text_lower = text.to_lowercase();
        for skill in self.skills.read().unwrap().values() {
            if skill.triggers.iter().any(|t| text_lower.contains(&t.to_lowercase())) {
                self.active.write().unwrap().insert(chat_id, skill.name.clone());
                return Some(skill.clone());
            }
        }
        None
    }

    pub fn reload(&self, dir: &Path) -> anyhow::Result<()> {
        let mut map = HashMap::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.path().extension().and_then(|s| s.to_str()) != Some("toml") {
                    continue;
                }
                let text = fs::read_to_string(entry.path())?;
                let skill: Skill = toml::from_str(&text)?;
                map.insert(skill.name.clone(), skill);
            }
        }
        *self.skills.write().unwrap() = map;
        Ok(())
    }
}
