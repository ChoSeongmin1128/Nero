use super::context::{Binding,Citation};
use serde::{Deserialize,Serialize};
use std::io::{Read,Write};
use std::path::{Path,PathBuf};

const MAX_RECORD: usize=8*1024*1024;
fn invalid()->String { "저장된 AI 대화를 확인하지 못했습니다.".into() }
pub fn id(value:&str)->Result<(),String> { if uuid::Uuid::parse_str(value).is_ok_and(|id|id.to_string()==value){Ok(())}else{Err("대화 정보를 확인해 주세요.".into())} }
fn plain(path:&Path,directory:bool)->Result<bool,String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta)=> {
            if meta.file_type().is_symlink() || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) { return Err(invalid()); }
            #[cfg(windows)] { use std::os::windows::fs::MetadataExt; if meta.file_attributes() & 0x400 != 0 {return Err(invalid());} }
            Ok(true)
        },
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(false),
        Err(_)=>Err(invalid()),
    }
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Message {
    pub id:String,pub request_id:String,pub role:String,pub text:String,pub status:String,
    #[serde(default)]pub citations:Vec<Citation>,
    #[serde(default)]pub error:Option<String>,
    #[serde(default)]pub provider:Option<String>,#[serde(default)]pub model_id:Option<String>,
    #[serde(default)]pub binding:Option<Binding>,
    #[serde(default)]pub saved_summary_id:Option<String>,#[serde(default)]pub note_saved:bool,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Conversation {
    pub format_version:u32,pub id:Option<String>,pub meeting_id:String,pub provider:Option<String>,pub model_id:Option<String>,pub messages:Vec<Message>,
    #[serde(default)]pub active_request_id:Option<String>,
}
impl Conversation {
    pub fn empty(meeting_id:&str)->Self { Self{format_version:1,id:None,meeting_id:meeting_id.into(),provider:None,model_id:None,messages:vec![],active_request_id:None} }
}
pub struct Store { root:PathBuf }
impl Store {
    pub fn new(root:PathBuf)->Self{Self{root}}
    fn directory(&self,create:bool)->Result<PathBuf,String>{
        if !plain(&self.root,true)?{return Err(invalid());}
        let directory=self.root.join("ai-conversations");
        if !plain(&directory,true)? && create {std::fs::create_dir(&directory).map_err(|_|invalid())?;}
        Ok(directory)
    }
    pub fn read(&self,meeting_id:&str)->Result<Conversation,String>{
        id(meeting_id)?;let directory=self.directory(false)?;
        if !plain(&directory,true)?{return Ok(Conversation::empty(meeting_id));}
        let path=directory.join(format!("{meeting_id}.json"));
        if !plain(&path,false)?{return Ok(Conversation::empty(meeting_id));}
        let mut raw=Vec::new();std::fs::File::open(path).map_err(|_|invalid())?.take(MAX_RECORD as u64+1).read_to_end(&mut raw).map_err(|_|invalid())?;
        if raw.len()>MAX_RECORD{return Err(invalid());}
        let value:Conversation=serde_json::from_slice(&raw).map_err(|_|invalid())?;
        if value.format_version!=1 || value.meeting_id!=meeting_id || value.id.as_deref().is_none_or(|value|id(value).is_err()) || value.messages.len()>400 || value.messages.iter().any(|message|id(&message.id).is_err() || id(&message.request_id).is_err() || !matches!(message.role.as_str(),"user"|"assistant") || !matches!(message.status.as_str(),"completed"|"streaming"|"failed"|"cancelled"|"interrupted")) {return Err(invalid());}
        Ok(value)
    }
    pub fn write(&self,value:&Conversation)->Result<(),String>{
        id(&value.meeting_id)?;id(value.id.as_deref().ok_or_else(invalid)?)?;
        let raw=serde_json::to_vec(value).map_err(|_|invalid())?;
        if raw.len()>MAX_RECORD || value.messages.len()>400{return Err("대화가 너무 깁니다. 새로운 대화가 필요합니다.".into());}
        let directory=self.directory(true)?;let path=directory.join(format!("{}.json",value.meeting_id));plain(&path,false)?;
        let mut file=tempfile::NamedTempFile::new_in(&directory).map_err(|_|invalid())?;
        file.write_all(&raw).and_then(|_|file.as_file().sync_all()).map_err(|_|invalid())?;
        plain(&path,false)?;file.persist(&path).map_err(|_|invalid())?;
        Ok(())
    }
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn read_never_initializes_conversations_and_failed_overwrite_preserves_data(){
  let tmp=tempfile::tempdir().unwrap();let store=Store::new(tmp.path().into());let meeting=uuid::Uuid::new_v4().to_string();
  assert!(store.read(&meeting).unwrap().id.is_none());assert!(!tmp.path().join("ai-conversations").exists());
  let mut value=Conversation::empty(&meeting);value.id=Some(uuid::Uuid::new_v4().to_string());store.write(&value).unwrap();let before=std::fs::read(tmp.path().join("ai-conversations").join(format!("{meeting}.json"))).unwrap();
  value.messages=vec![Message{id:uuid::Uuid::new_v4().to_string(),request_id:uuid::Uuid::new_v4().to_string(),role:"assistant".into(),text:"x".repeat(MAX_RECORD),status:"failed".into(),citations:vec![],error:None,provider:None,model_id:None,binding:None,saved_summary_id:None,note_saved:false}];
  assert!(store.write(&value).is_err());assert_eq!(std::fs::read(tmp.path().join("ai-conversations").join(format!("{meeting}.json"))).unwrap(),before);
 }
 #[cfg(unix)]#[test]fn linked_conversation_directories_are_rejected(){let root=tempfile::tempdir().unwrap();let outside=tempfile::tempdir().unwrap();std::os::unix::fs::symlink(outside.path(),root.path().join("ai-conversations")).unwrap();assert!(Store::new(root.path().into()).read(&uuid::Uuid::new_v4().to_string()).is_err());assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(),0);}
}
