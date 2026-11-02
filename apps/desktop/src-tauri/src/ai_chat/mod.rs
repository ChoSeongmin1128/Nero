mod context;
mod provider;
mod store;
pub use provider::{ChatModel,Models};
pub use store::Conversation;
use crate::ai_accounts::{AiAccounts,Provider};
use crate::notion_context::NotionContextService;
use context::Binding;
use nero_core::CoreHandle;
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool,Ordering},Arc,Mutex};
use store::Message;

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct SendRequest {
 pub request_id:String,pub conversation_id:Option<String>,pub meeting_id:String,
 pub include_meeting:bool,pub account_id:String,pub model_id:String,pub message:String,
 #[serde(default)]pub source_ids:Vec<String>,pub template_id:Option<String>,
}
#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub struct ChatEvent {
 pub request_id:String,pub conversation_id:String,pub meeting_id:String,pub message_id:String,
 #[serde(rename="type")]pub kind:String,pub text:Option<String>,pub error:Option<String>,
 pub citations:Vec<context::Citation>,pub sequence:u64,
}
#[derive(Serialize)]#[serde(rename_all="camelCase")]
pub struct Started { pub conversation_id:String,pub request_id:String }
struct Active { conversation_id:String,meeting_id:String,message_id:String,account_id:String,cancel:Arc<AtomicBool>,partial:Arc<Mutex<String>> }
pub struct AiChat {
 root:PathBuf,core:CoreHandle,pub accounts:Arc<AiAccounts>,pub notion:Option<Arc<NotionContextService>>,
 store:Mutex<store::Store>,running:Mutex<HashMap<String,Active>>,
}
fn provider_id(provider:Provider)->&'static str {match provider{Provider::ChatGpt=>"chatgpt",Provider::Claude=>"claude"}}
impl AiChat {
 pub fn new(root:PathBuf,core:CoreHandle,accounts:Arc<AiAccounts>,notion:Option<Arc<NotionContextService>>)->Self {
  Self{store:Mutex::new(store::Store::new(root.clone())),root,core,accounts,notion,running:Mutex::new(HashMap::new())}
 }
 pub fn is_busy(&self)->bool{!self.running.lock().unwrap().is_empty()}
 pub fn cancel_all(&self){for active in self.running.lock().unwrap().values(){active.cancel.store(true,Ordering::Release);}}
 pub fn cancel_account(&self,id:&str){for active in self.running.lock().unwrap().values().filter(|active|active.account_id==id){active.cancel.store(true,Ordering::Release);}}
 pub fn cancel(&self,request:&str)->Result<(),String>{store::id(request)?;if let Some(active)=self.running.lock().unwrap().get(request){active.cancel.store(true,Ordering::Release);}Ok(())}
 pub fn read(&self,meeting:&str)->Result<Conversation,String>{
  self.require_meeting(meeting)?;let mut conversation=self.store.lock().unwrap().read(meeting)?;let running=self.running.lock().unwrap();
  for message in &mut conversation.messages {if message.status=="streaming" {
   if let Some(active)=running.get(&message.request_id){message.text=active.partial.lock().unwrap().clone();conversation.active_request_id=Some(message.request_id.clone());}
   else{message.status="interrupted".into();message.error=Some("답변이 중단되었습니다. 다시 시도해 주세요.".into());conversation.active_request_id=None;}
  }}Ok(conversation)
 }
 fn require_meeting(&self,id:&str)->Result<(),String>{
  store::id(id)?;if self.core.list_meetings().map_err(|e|e.to_string())?.iter().any(|meeting|meeting.id==id && meeting.deleted_at.is_none()){Ok(())}else{Err("회의를 찾을 수 없습니다.".into())}
 }
 pub async fn models(&self,id:&str)->Result<Models,String>{
  let session=self.accounts.subscription_session(id).await?;
  provider::models(&provider::ProviderAccess{provider:provider_id(session.provider).into(),access_token:session.access_token().into()}).await
 }
 pub fn start(self:&Arc<Self>,request:SendRequest,emit:Arc<dyn Fn(ChatEvent)+Send+Sync>)->Result<Started,String>{
  store::id(&request.request_id)?;self.require_meeting(&request.meeting_id)?;
  if request.message.trim().is_empty() || request.message.len()>32*1024 || request.source_ids.len()>16 || request.model_id.is_empty() || request.model_id.len()>128 {return Err("질문과 선택한 자료를 확인해 주세요.".into());}
  let account=self.accounts.snapshot()?.accounts.into_iter().find(|account|account.id==request.account_id && account.can_invoke).ok_or("먼저 AI 계정을 연결해 주세요.")?;
  let documents=if request.source_ids.is_empty(){vec![]}else{self.notion.as_ref().ok_or("Notion 연결을 준비하지 못했습니다.")?.context_bundle(&request.source_ids).map_err(|e|e.to_string())?.sources.into_iter().map(|source|serde_json::to_value(source).map_err(|_|"자료를 확인하지 못했습니다.".to_string())).collect::<Result<Vec<_>,_>>()?};
  if documents.iter().any(|source|source["truncated"]==true){return Err("전체 내용을 읽지 못한 자료가 있습니다. 자료를 다시 확인해 주세요.".into());}
  let prepared=context::prepare(&self.core,&self.root,&request.meeting_id,request.include_meeting,documents,request.template_id.as_deref())?;
  let store=self.store.lock().unwrap();let mut conversation=store.read(&request.meeting_id)?;let mut running=self.running.lock().unwrap();
  if let Some(existing)=conversation.messages.iter().find(|message|message.role=="user" && message.request_id==request.request_id){
   if existing.text!=request.message{return Err("같은 요청의 질문이 변경되었습니다.".into());}
   return Ok(Started{conversation_id:conversation.id.unwrap(),request_id:request.request_id});
  }
  if running.values().any(|active|active.meeting_id==request.meeting_id){return Err("이 회의의 답변을 기다리거나 중지해 주세요.".into());}
  if running.len()>=4{return Err("진행 중인 답변이 끝난 뒤 다시 시도해 주세요.".into());}
  if conversation.id!=request.conversation_id{return Err("대화가 변경되었습니다. 다시 열어 주세요.".into());}
  for message in &mut conversation.messages{if message.status=="streaming"{message.status="interrupted".into();}}
  let conversation_id=conversation.id.get_or_insert_with(||uuid::Uuid::new_v4().to_string()).clone();let message_id=uuid::Uuid::new_v4().to_string();
  let mut history=conversation.messages.iter().filter(|message|message.status=="completed").map(|message|{
   let text=nero_core::presentation::format_summary(&message.text,nero_core::DocumentFormat::Md,false,false,None);
   json!({"role":message.role,"content":text})
  }).collect::<Vec<Value>>();
  history.push(json!({"role":"user","content":format!("{}\n\n참고 자료(JSON 데이터):\n{}",request.message,prepared.input)}));
  if serde_json::to_vec(&history).map_err(|_|"대화를 준비하지 못했습니다.")?.len()>2*1024*1024{return Err("대화와 자료가 너무 많습니다. 선택한 자료를 줄여 주세요.".into());}
  conversation.provider=Some(provider_id(account.provider).into());conversation.model_id=Some(request.model_id.clone());conversation.active_request_id=Some(request.request_id.clone());
  conversation.messages.push(Message{id:uuid::Uuid::new_v4().to_string(),request_id:request.request_id.clone(),role:"user".into(),text:request.message.clone(),status:"completed".into(),citations:vec![],error:None,provider:None,model_id:None,binding:None,saved_summary_id:None,note_saved:false});
  conversation.messages.push(Message{id:message_id.clone(),request_id:request.request_id.clone(),role:"assistant".into(),text:String::new(),status:"streaming".into(),citations:vec![],error:None,provider:Some(provider_id(account.provider).into()),model_id:Some(request.model_id.clone()),binding:Some(prepared.binding),saved_summary_id:None,note_saved:false});
  store.write(&conversation)?;
  let cancel=Arc::new(AtomicBool::new(false));let partial=Arc::new(Mutex::new(String::new()));
  running.insert(request.request_id.clone(),Active{conversation_id:conversation_id.clone(),meeting_id:request.meeting_id.clone(),message_id:message_id.clone(),account_id:request.account_id.clone(),cancel:cancel.clone(),partial:partial.clone()});
  drop(running);drop(store);
  let started=Started{conversation_id:conversation_id.clone(),request_id:request.request_id.clone()};let this=self.clone();
  tauri::async_runtime::spawn(async move{
   let mut sequence=0;
   let result=async{
    let session=this.accounts.subscription_session_with_cancel(&request.account_id,cancel.clone()).await?;
    let access=provider::ProviderAccess{provider:provider_id(session.provider).into(),access_token:session.access_token().into()};
    let models=provider::models(&access).await?;
    if !models.models.iter().any(|model|model.id==request.model_id){return Err("이 AI 계정에서 선택한 모델을 사용할 수 없습니다.".into());}
    provider::reply(&access,&request.model_id,&prepared.instructions,history,cancel.clone(),|delta|{
     partial.lock().unwrap().push_str(delta);sequence+=1;
     emit(ChatEvent{request_id:request.request_id.clone(),conversation_id:conversation_id.clone(),meeting_id:request.meeting_id.clone(),message_id:message_id.clone(),kind:"delta".into(),text:Some(delta.into()),error:None,citations:vec![],sequence});
    }).await
   }.await;
   let event=this.finish(&request.request_id,result,cancel.load(Ordering::Acquire),sequence+1);
   if let Ok(event)=event{emit(event);}else{emit(ChatEvent{request_id:request.request_id.clone(),conversation_id,meeting_id:request.meeting_id,message_id,kind:"failed".into(),text:Some(partial.lock().unwrap().clone()),error:Some("AI 대화를 저장하지 못했습니다. 다시 열어 확인해 주세요.".into()),citations:vec![],sequence:sequence+1});}
  });Ok(started)
 }
 fn finish(&self,request_id:&str,result:Result<String,String>,cancelled:bool,sequence:u64)->Result<ChatEvent,String>{
  let store=self.store.lock().unwrap();let mut running=self.running.lock().unwrap();let active=running.get(request_id).ok_or("진행 중인 대화를 찾을 수 없습니다.")?;
  let mut conversation=store.read(&active.meeting_id)?;let message=conversation.messages.iter_mut().find(|message|message.id==active.message_id).ok_or("답변을 찾을 수 없습니다.")?;
  let result=if cancelled{Err("취소했습니다.".into())}else{result};
  match result.and_then(|text|context::cited(&text,&message.binding.as_ref().ok_or("답변 자료를 확인하지 못했습니다.")?.citations).map(|citations|(text,citations))){
   Ok((text,citations))=>{message.text=text;message.citations=citations;message.status="completed".into();message.error=None;},
   Err(error)=>{message.text=active.partial.lock().unwrap().clone();message.status=if cancelled{"cancelled"}else{"failed"}.into();message.error=Some(error);}
  }
  conversation.active_request_id=None;
  let event=ChatEvent{request_id:request_id.into(),conversation_id:active.conversation_id.clone(),meeting_id:active.meeting_id.clone(),message_id:message.id.clone(),kind:message.status.clone(),text:Some(message.text.clone()),error:message.error.clone(),citations:message.citations.clone(),sequence};
  let result=store.write(&conversation);running.remove(request_id);result?;Ok(event)
 }
 pub fn save_summary(&self,conversation_id:&str,message_id:&str,meeting_id:&str)->Result<Value,String>{
  self.apply_answer(conversation_id,message_id,meeting_id,true)
 }
 pub fn append_note(&self,conversation_id:&str,message_id:&str,meeting_id:&str)->Result<Value,String>{
  self.apply_answer(conversation_id,message_id,meeting_id,false)
 }
 fn apply_answer(&self,conversation_id:&str,message_id:&str,meeting_id:&str,summary:bool)->Result<Value,String>{
  self.require_meeting(meeting_id)?;store::id(conversation_id)?;store::id(message_id)?;
  let store=self.store.lock().unwrap();let mut conversation=store.read(meeting_id)?;
  if conversation.id.as_deref()!=Some(conversation_id){return Err("대화를 다시 확인해 주세요.".into());}
  let message=conversation.messages.iter_mut().find(|message|message.id==message_id && message.role=="assistant" && message.status=="completed").ok_or("완성된 답변을 선택해 주세요.")?;
  let binding:Binding=message.binding.clone().ok_or("답변 자료를 확인하지 못했습니다.")?;
  if binding.meeting_id!=meeting_id{return Err("다른 회의의 답변을 저장할 수 없습니다.".into());}
  if summary && message.saved_summary_id.is_some(){return Ok(json!({"summaryId":message.saved_summary_id}));}
  if !summary && message.note_saved{return Ok(json!({"applied":true}));}
  let text=context::summary_text(&message.text,&message.citations);
  let result=if summary{
   let context=binding.summary_context.ok_or("음성 기록을 포함한 답변만 회의 요약으로 저장할 수 있습니다.")?;
   let saved=self.core.import_summary(meeting_id,context,text,context::producer(message.provider.as_deref().unwrap_or("AI"),message.model_id.as_deref().unwrap_or("unknown"),binding.template)).map_err(|e|e.to_string())?;
   message.saved_summary_id=Some(saved.id.clone());json!({"summaryId":saved.id})
  }else{
   let text=nero_core::presentation::format_summary(&text,nero_core::DocumentFormat::Md,false,false,None);
   self.core.append_meeting_note(meeting_id,&binding.meeting_version,binding.summary_context,&text).map_err(|e|e.to_string())?;message.note_saved=true;json!({"applied":true})
  };
  store.write(&conversation)?;Ok(result)
 }
}
