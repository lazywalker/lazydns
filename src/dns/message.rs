//! DNS message implementation
//!
//! This module implements the DNS message structure as defined in RFC 1035.
//! A DNS message consists of a header and four sections: question, answer,
//! authority, and additional.

use super::question::Question;
use super::record::ResourceRecord;
use super::types::{OpCode, ResponseCode};
use std::fmt;

/// DNS message
///
/// Represents a complete DNS message including header and all sections.
/// This structure can represent both DNS queries and responses.
///
/// # Example
///
/// ```
/// use lazydns::dns::{Message, Question, RecordType, RecordClass};
///
/// let mut message = Message::new();
/// message.set_id(1234);
/// message.set_query(true);
/// message.add_question(Question::new(
///     "example.com",
///     RecordType::A,
///     RecordClass::IN,
/// ));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    // Header fields
    /// Message ID
    id: u16,
    /// Query/Response flag (false = query, true = response)
    qr: bool,
    /// Operation code
    opcode: OpCode,
    /// Authoritative answer flag
    aa: bool,
    /// Truncation flag
    tc: bool,
    /// Recursion desired flag
    rd: bool,
    /// Recursion available flag
    ra: bool,
    /// Authenticated Data flag (RFC 6840 §5.7)
    ad: bool,
    /// Checking Disabled flag (RFC 4035)
    cd: bool,
    /// Response code
    rcode: ResponseCode,

    // Message sections
    /// Question section
    questions: Vec<Question>,
    /// Answer section
    answers: Vec<ResourceRecord>,
    /// Authority section
    authority: Vec<ResourceRecord>,
    /// Additional section
    additional: Vec<ResourceRecord>,
}

impl Message {
    /// Create a new DNS message with default values
    ///
    /// The message is initialized as a query (QR=0) with QUERY opcode
    /// and NOERROR response code.
    pub fn new() -> Self {
        Self {
            id: 0,
            qr: false,
            opcode: OpCode::Query,
            aa: false,
            tc: false,
            rd: true,
            ra: false,
            ad: false,
            cd: false,
            rcode: ResponseCode::NoError,
            questions: Vec::new(),
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
    }

    /// Get the message ID
    pub fn id(&self) -> u16 {
        self.id
    }

    /// Set the message ID
    pub fn set_id(&mut self, id: u16) {
        self.id = id;
    }

    /// Check if this is a query (false) or response (true)
    pub fn is_response(&self) -> bool {
        self.qr
    }

    /// Set whether this is a query or response
    pub fn set_query(&mut self, is_query: bool) {
        self.qr = !is_query;
    }

    /// Set whether this is a response
    pub fn set_response(&mut self, is_response: bool) {
        self.qr = is_response;
    }

    /// Get the operation code
    pub fn opcode(&self) -> OpCode {
        self.opcode
    }

    /// Set the operation code
    pub fn set_opcode(&mut self, opcode: OpCode) {
        self.opcode = opcode;
    }

    /// Check if authoritative answer flag is set
    pub fn is_authoritative(&self) -> bool {
        self.aa
    }

    /// Set the authoritative answer flag
    pub fn set_authoritative(&mut self, aa: bool) {
        self.aa = aa;
    }

    /// Check if truncation flag is set
    pub fn is_truncated(&self) -> bool {
        self.tc
    }

    /// Set the truncation flag
    pub fn set_truncated(&mut self, tc: bool) {
        self.tc = tc;
    }

    /// Check if recursion desired flag is set
    pub fn recursion_desired(&self) -> bool {
        self.rd
    }

    /// Set the recursion desired flag
    pub fn set_recursion_desired(&mut self, rd: bool) {
        self.rd = rd;
    }

    /// Check if recursion available flag is set
    pub fn recursion_available(&self) -> bool {
        self.ra
    }

    /// Set the recursion available flag
    pub fn set_recursion_available(&mut self, ra: bool) {
        self.ra = ra;
    }

    /// Check if the Authenticated Data flag is set (RFC 6840 §5.7)
    pub fn authentic_data(&self) -> bool {
        self.ad
    }

    /// Set the Authenticated Data flag
    pub fn set_authentic_data(&mut self, ad: bool) {
        self.ad = ad;
    }

    /// Check if the Checking Disabled flag is set (RFC 4035)
    pub fn checking_disabled(&self) -> bool {
        self.cd
    }

    /// Set the Checking Disabled flag
    pub fn set_checking_disabled(&mut self, cd: bool) {
        self.cd = cd;
    }

    /// Get the response code
    pub fn response_code(&self) -> ResponseCode {
        self.rcode
    }

    /// Set the response code
    pub fn set_response_code(&mut self, rcode: ResponseCode) {
        self.rcode = rcode;
    }

    /// Get the questions
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// Get mutable questions
    pub fn questions_mut(&mut self) -> &mut Vec<Question> {
        &mut self.questions
    }

    /// Add a question to the message
    pub fn add_question(&mut self, question: Question) {
        self.questions.push(question);
    }

    /// Get the answers
    pub fn answers(&self) -> &[ResourceRecord] {
        &self.answers
    }

    /// Get mutable answers
    pub fn answers_mut(&mut self) -> &mut Vec<ResourceRecord> {
        &mut self.answers
    }

    /// Add an answer to the message
    pub fn add_answer(&mut self, answer: ResourceRecord) {
        self.answers.push(answer);
    }

    /// Get the authority records
    pub fn authority(&self) -> &[ResourceRecord] {
        &self.authority
    }

    /// Get mutable authority records
    pub fn authority_mut(&mut self) -> &mut Vec<ResourceRecord> {
        &mut self.authority
    }

    /// Add an authority record to the message
    pub fn add_authority(&mut self, authority: ResourceRecord) {
        self.authority.push(authority);
    }

    /// Get the additional records
    pub fn additional(&self) -> &[ResourceRecord] {
        &self.additional
    }

    /// Get mutable additional records
    pub fn additional_mut(&mut self) -> &mut Vec<ResourceRecord> {
        &mut self.additional
    }

    /// Add an additional record to the message
    pub fn add_additional(&mut self, additional: ResourceRecord) {
        self.additional.push(additional);
    }

    /// Get the count of questions
    pub fn question_count(&self) -> usize {
        self.questions.len()
    }

    /// Get the count of answer records
    pub fn answer_count(&self) -> usize {
        self.answers.len()
    }

    /// Get the count of authority records
    pub fn authority_count(&self) -> usize {
        self.authority.len()
    }

    /// Get the count of additional records
    pub fn additional_count(&self) -> usize {
        self.additional.len()
    }

    /// Clear all questions
    pub fn clear_questions(&mut self) {
        self.questions.clear();
    }

    /// Clear all answers
    pub fn clear_answers(&mut self) {
        self.answers.clear();
    }

    /// Clear all authority records
    pub fn clear_authority(&mut self) {
        self.authority.clear();
    }

    /// Clear all additional records
    pub fn clear_additional(&mut self) {
        self.additional.clear();
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, ";; DNS Message")?;
        writeln!(f, ";; ID: {}", self.id)?;
        writeln!(
            f,
            ";; QR: {} ({})",
            self.qr,
            if self.qr { "Response" } else { "Query" }
        )?;
        writeln!(f, ";; OPCODE: {:?}", self.opcode)?;
        writeln!(f, ";; RCODE: {}", self.rcode)?;
        writeln!(
            f,
            ";; Flags: AA={} TC={} RD={} RA={}",
            self.aa, self.tc, self.rd, self.ra
        )?;
        writeln!(
            f,
            ";; Counts: QUESTION={} ANSWER={} AUTHORITY={} ADDITIONAL={}",
            self.question_count(),
            self.answer_count(),
            self.authority_count(),
            self.additional_count()
        )?;

        if !self.questions.is_empty() {
            writeln!(f, "\n;; QUESTION SECTION:")?;
            for question in &self.questions {
                writeln!(f, "{}", question)?;
            }
        }

        if !self.answers.is_empty() {
            writeln!(f, "\n;; ANSWER SECTION:")?;
            for answer in &self.answers {
                writeln!(f, "{}", answer)?;
            }
        }

        if !self.authority.is_empty() {
            writeln!(f, "\n;; AUTHORITY SECTION:")?;
            for auth in &self.authority {
                writeln!(f, "{}", auth)?;
            }
        }

        if !self.additional.is_empty() {
            writeln!(f, "\n;; ADDITIONAL SECTION:")?;
            for add in &self.additional {
                writeln!(f, "{}", add)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{RData, RecordClass, RecordType};
    use std::net::Ipv4Addr;

    #[test]
    fn test_message_creation() {
        let message = Message::new();

        assert_eq!(message.id(), 0);
        assert!(!message.is_response());
        assert_eq!(message.opcode(), OpCode::Query);
        assert_eq!(message.response_code(), ResponseCode::NoError);
        assert!(message.recursion_desired());
        assert!(!message.recursion_available());
    }

    #[test]
    fn test_message_id() {
        let mut message = Message::new();
        message.set_id(12345);
        assert_eq!(message.id(), 12345);
    }

    #[test]
    fn test_message_flags() {
        let mut message = Message::new();

        message.set_response(true);
        assert!(message.is_response());

        message.set_authoritative(true);
        assert!(message.is_authoritative());

        message.set_truncated(true);
        assert!(message.is_truncated());

        message.set_recursion_desired(false);
        assert!(!message.recursion_desired());

        message.set_recursion_available(true);
        assert!(message.recursion_available());
    }

    #[test]
    fn test_add_question() {
        let mut message = Message::new();
        let question = Question::new("example.com", RecordType::A, RecordClass::IN);

        message.add_question(question);
        assert_eq!(message.question_count(), 1);
        assert_eq!(message.questions()[0].qname(), "example.com");
    }

    #[test]
    fn test_add_answer() {
        let mut message = Message::new();
        let answer = ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            3600,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        );

        message.add_answer(answer);
        assert_eq!(message.answer_count(), 1);
        assert_eq!(message.answers()[0].name(), "example.com");
    }

    #[test]
    fn test_clear_sections() {
        let mut message = Message::new();

        message.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        message.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            3600,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        ));

        assert_eq!(message.question_count(), 1);
        assert_eq!(message.answer_count(), 1);

        message.clear_questions();
        message.clear_answers();

        assert_eq!(message.question_count(), 0);
        assert_eq!(message.answer_count(), 0);
    }

    #[test]
    fn test_response_code() {
        let mut message = Message::new();

        message.set_response_code(ResponseCode::NXDomain);
        assert_eq!(message.response_code(), ResponseCode::NXDomain);

        message.set_response_code(ResponseCode::ServFail);
        assert_eq!(message.response_code(), ResponseCode::ServFail);
    }

    #[test]
    fn test_opcode() {
        let mut message = Message::new();

        message.set_opcode(OpCode::Update);
        assert_eq!(message.opcode(), OpCode::Update);
    }

    #[test]
    fn test_complete_query_message() {
        let mut message = Message::new();
        message.set_id(1234);
        message.set_query(true);
        message.set_recursion_desired(true);
        message.add_question(Question::new(
            "www.example.com",
            RecordType::A,
            RecordClass::IN,
        ));

        assert_eq!(message.id(), 1234);
        assert!(!message.is_response());
        assert!(message.recursion_desired());
        assert_eq!(message.question_count(), 1);
    }

    #[test]
    fn test_complete_response_message() {
        let mut message = Message::new();
        message.set_id(1234);
        message.set_response(true);
        message.set_authoritative(true);
        message.set_recursion_available(true);
        message.set_response_code(ResponseCode::NoError);

        message.add_question(Question::new(
            "www.example.com",
            RecordType::A,
            RecordClass::IN,
        ));

        message.add_answer(ResourceRecord::new(
            "www.example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(93, 184, 216, 34)),
        ));

        assert_eq!(message.id(), 1234);
        assert!(message.is_response());
        assert!(message.is_authoritative());
        assert!(message.recursion_available());
        assert_eq!(message.response_code(), ResponseCode::NoError);
        assert_eq!(message.question_count(), 1);
        assert_eq!(message.answer_count(), 1);
    }

    #[test]
    fn test_authority_section() {
        let mut message = Message::new();
        message.add_authority(ResourceRecord::new(
            "example.com",
            RecordType::NS,
            RecordClass::IN,
            86400,
            RData::NS("ns1.example.com".to_string()),
        ));

        assert_eq!(message.authority_count(), 1);
        assert_eq!(message.authority().len(), 1);
        assert_eq!(message.authority()[0].name(), "example.com");
    }

    #[test]
    fn test_additional_section() {
        let mut message = Message::new();
        message.add_additional(ResourceRecord::new(
            "ns1.example.com",
            RecordType::A,
            RecordClass::IN,
            3600,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        ));

        assert_eq!(message.additional_count(), 1);
        assert_eq!(message.additional().len(), 1);
    }

    #[test]
    fn test_message_clone() {
        let mut message = Message::new();
        message.set_id(5678);
        message.add_question(Question::new("test.com", RecordType::A, RecordClass::IN));

        let cloned = message.clone();
        assert_eq!(cloned.id(), 5678);
        assert_eq!(cloned.question_count(), 1);
    }

    #[test]
    fn test_message_debug() {
        let message = Message::new();
        let debug_str = format!("{:?}", message);
        assert!(debug_str.contains("Message"));
    }

    #[test]
    fn test_questions_mut() {
        let mut message = Message::new();
        message.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let questions = message.questions_mut();
        assert_eq!(questions.len(), 1);
        questions.clear();
        assert_eq!(message.question_count(), 0);
    }

    #[test]
    fn test_answers_mut() {
        let mut message = Message::new();
        message.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(1, 2, 3, 4)),
        ));

        let answers = message.answers_mut();
        assert_eq!(answers.len(), 1);
    }

    #[test]
    fn test_authority_mut() {
        let mut message = Message::new();
        let auth = message.authority_mut();
        auth.push(ResourceRecord::new(
            "example.com",
            RecordType::NS,
            RecordClass::IN,
            86400,
            RData::NS("ns1.example.com".to_string()),
        ));
        assert_eq!(message.authority_count(), 1);
    }

    #[test]
    fn test_additional_mut() {
        let mut message = Message::new();
        let additional = message.additional_mut();
        additional.push(ResourceRecord::new(
            "ns1.example.com",
            RecordType::A,
            RecordClass::IN,
            3600,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        ));
        assert_eq!(message.additional_count(), 1);
    }

    #[test]
    fn test_clear_authority() {
        let mut message = Message::new();
        message.add_authority(ResourceRecord::new(
            "example.com",
            RecordType::NS,
            RecordClass::IN,
            86400,
            RData::NS("ns1.example.com".to_string()),
        ));

        assert_eq!(message.authority_count(), 1);
        message.clear_authority();
        assert_eq!(message.authority_count(), 0);
    }

    #[test]
    fn test_clear_additional() {
        let mut message = Message::new();
        message.add_additional(ResourceRecord::new(
            "ns1.example.com",
            RecordType::A,
            RecordClass::IN,
            3600,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        ));

        assert_eq!(message.additional_count(), 1);
        message.clear_additional();
        assert_eq!(message.additional_count(), 0);
    }
}
