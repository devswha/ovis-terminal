//! Rules a released-baseline comparison can find broken.

/// Rule a change violates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BreakRule {
    /// A released method is gone.
    MethodRemoved,
    /// A released parameter is gone.
    ParamRemoved,
    /// An optional parameter became required.
    ParamNewlyRequired,
    /// A new parameter is required.
    RequiredParamAdded,
    /// An input accepts fewer JSON types than before.
    InputTypeNarrowed,
    /// An input accepts fewer values than before, including a free value becoming an enum.
    InputValuesNarrowed,
    /// An input numeric, length, item or property bound became tighter.
    InputBoundNarrowed,
    /// An input pattern was added or changed.
    InputPatternChanged,
    /// An input object or union lost a property or variant.
    InputShapeRemoved,
    /// An input object property became required.
    InputPropertyNewlyRequired,
    /// An input object stopped accepting additional properties.
    InputClosed,
    /// An output can now carry a JSON type it never carried.
    OutputTypeChanged,
    /// An output lost its declared type.
    OutputTypeLost,
    /// A released output property is gone.
    OutputPropertyRemoved,
    /// A required output property became optional.
    OutputNoLongerRequired,
    /// A released output union variant has no counterpart.
    OutputVariantRemoved,
    /// A closed output enum gained a value clients cannot decode.
    ClosedOutputEnumGrew,
    /// A released event type is gone.
    EventRemoved,
    /// A released notification is gone.
    NotificationRemoved,
}

impl BreakRule {
    /// Returns the stable machine token of this rule.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::MethodRemoved => "method_removed",
            Self::ParamRemoved => "param_removed",
            Self::ParamNewlyRequired => "param_newly_required",
            Self::RequiredParamAdded => "required_param_added",
            Self::InputTypeNarrowed => "input_type_narrowed",
            Self::InputValuesNarrowed => "input_values_narrowed",
            Self::InputBoundNarrowed => "input_bound_narrowed",
            Self::InputPatternChanged => "input_pattern_changed",
            Self::InputShapeRemoved => "input_shape_removed",
            Self::InputPropertyNewlyRequired => "input_property_newly_required",
            Self::InputClosed => "input_closed",
            Self::OutputTypeChanged => "output_type_changed",
            Self::OutputTypeLost => "output_type_lost",
            Self::OutputPropertyRemoved => "output_property_removed",
            Self::OutputNoLongerRequired => "output_no_longer_required",
            Self::OutputVariantRemoved => "output_variant_removed",
            Self::ClosedOutputEnumGrew => "closed_output_enum_grew",
            Self::EventRemoved => "event_removed",
            Self::NotificationRemoved => "notification_removed",
        }
    }
}
